//! The build pipeline: aapt2 → javac → d8 → package → zipalign → apksigner.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::config::AppConfig;
use crate::project;
use crate::toolchain::{Jdk, Toolchain};

pub struct BuildFlags {
    pub keep_build: Option<PathBuf>,
    pub verbose: bool,
    pub install: bool,
}

const DEFAULT_KEY_ALIAS: &str = "url2apk";
const DEFAULT_KEY_PASSWORD: &str = "android";
const STORE_PASS_ENV: &str = "URL2APK_INTERNAL_KS_PASS";
const KEY_PASS_ENV: &str = "URL2APK_INTERNAL_KEY_PASS";

struct Step {
    n: usize,
    total: usize,
}

impl Step {
    fn next(&mut self, msg: &str) {
        self.n += 1;
        println!("[{}/{}] {msg}", self.n, self.total);
    }
}

pub fn build(cfg: &AppConfig, flags: &BuildFlags) -> Result<PathBuf> {
    let tc = Toolchain::detect(cfg.sdk.as_deref(), cfg.jdk.as_deref(), cfg.compile_sdk)?;
    let compile_sdk = tc.platform.api;
    let target_sdk = cfg.target_sdk.unwrap_or(compile_sdk);
    if cfg.min_sdk > target_sdk {
        bail!("--min-sdk ({}) is higher than the target SDK ({target_sdk})", cfg.min_sdk);
    }
    if flags.install && tc.adb.is_none() {
        bail!("--install needs adb, but platform-tools is not installed (sdkmanager \"platform-tools\")");
    }

    println!("Building {} ({}) from {}", cfg.name, cfg.package, cfg.url);
    if flags.verbose {
        println!(
            "  JDK: {}\n  SDK: {}\n  build-tools {} | compile SDK {compile_sdk} | target SDK {target_sdk} | min SDK {}",
            tc.jdk.home.display(),
            tc.sdk.display(),
            tc.build_tools.version,
            cfg.min_sdk
        );
    }

    // Workspace: a temp dir that is deleted afterwards, unless --keep-build.
    let _temp;
    let ws: PathBuf = match &flags.keep_build {
        Some(dir) => {
            if dir.exists() {
                std::fs::remove_dir_all(dir)
                    .with_context(|| format!("cannot clean build dir {}", dir.display()))?;
            }
            std::fs::create_dir_all(dir)?;
            dir.clone()
        }
        None => {
            let t = tempfile::Builder::new().prefix("url-to-apk-").tempdir()?;
            let p = t.path().to_path_buf();
            _temp = t;
            p
        }
    };
    let v = flags.verbose;
    let mut step = Step { n: 0, total: if flags.install { 8 } else { 7 } };

    step.next("Generating Android project");
    let proj = project::generate(cfg, &ws.join("project"), compile_sdk)?;

    step.next("Compiling resources (aapt2)");
    let compiled = ws.join("compiled-res.zip");
    run(
        Command::new(&tc.build_tools.aapt2).arg("compile").arg("--dir").arg(&proj.res).arg("-o").arg(&compiled),
        "aapt2 compile",
        v,
    )?;
    let base_apk = ws.join("base.apk");
    run(
        Command::new(&tc.build_tools.aapt2)
            .arg("link")
            .arg("-o")
            .arg(&base_apk)
            .arg("-I")
            .arg(&tc.platform.android_jar)
            .arg("--manifest")
            .arg(&proj.manifest)
            .args(["--min-sdk-version", &cfg.min_sdk.to_string()])
            .args(["--target-sdk-version", &target_sdk.to_string()])
            .args(["--version-code", &cfg.version_code.to_string()])
            .args(["--version-name", &cfg.version_name])
            .arg(&compiled),
        "aapt2 link",
        v,
    )?;

    step.next("Compiling Java (javac)");
    let classes = ws.join("classes");
    std::fs::create_dir_all(&classes)?;
    javac(&tc, &proj.java_sources, &classes, v)?;

    step.next("Converting to DEX (d8)");
    let dex_dir = ws.join("dex");
    std::fs::create_dir_all(&dex_dir)?;
    let mut class_files = Vec::new();
    collect_files(&classes, "class", &mut class_files)?;
    run(
        Command::new(&tc.jdk.java)
            .arg("-cp")
            .arg(&tc.build_tools.d8_jar)
            .arg("com.android.tools.r8.D8")
            .arg("--release")
            .args(["--min-api", &cfg.min_sdk.to_string()])
            .arg("--lib")
            .arg(&tc.platform.android_jar)
            .arg("--output")
            .arg(&dex_dir)
            .args(&class_files),
        "d8",
        v,
    )?;

    step.next("Packaging and aligning APK");
    let unaligned = ws.join("unaligned.apk");
    add_dex(&base_apk, &dex_dir, &unaligned)?;
    let aligned = ws.join("aligned.apk");
    run(
        Command::new(&tc.build_tools.zipalign).args(["-f", "-p", "4"]).arg(&unaligned).arg(&aligned),
        "zipalign",
        v,
    )?;

    step.next("Signing APK (apksigner)");
    let signing = resolve_signing(cfg, &tc.jdk, v)?;
    let output = output_path(cfg);
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    run(
        Command::new(&tc.jdk.java)
            .arg("-jar")
            .arg(&tc.build_tools.apksigner_jar)
            .arg("sign")
            .arg("--ks")
            .arg(&signing.keystore)
            .args(["--ks-key-alias", &signing.alias])
            .args(["--ks-pass", &format!("env:{STORE_PASS_ENV}")])
            .args(["--key-pass", &format!("env:{KEY_PASS_ENV}")])
            .arg("--out")
            .arg(&output)
            .arg(&aligned)
            .env(STORE_PASS_ENV, &signing.store_password)
            .env(KEY_PASS_ENV, &signing.key_password),
        "apksigner sign",
        v,
    )
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("UnrecoverableKey") || msg.contains("BadPadding") || msg.contains("password") {
            anyhow::anyhow!("signing failed: wrong keystore or key password for {}", signing.keystore.display())
        } else if msg.contains("No key with alias") || msg.contains("does not contain key") {
            anyhow::anyhow!("signing failed: alias `{}` not found in {}", signing.alias, signing.keystore.display())
        } else {
            e
        }
    })?;
    // apksigner may leave an .idsig next to the output for v4 signing; it's not needed.
    let _ = std::fs::remove_file(PathBuf::from(format!("{}.idsig", output.display())));

    step.next("Verifying signature");
    run(
        Command::new(&tc.jdk.java).arg("-jar").arg(&tc.build_tools.apksigner_jar).arg("verify").arg(&output),
        "apksigner verify",
        v,
    )?;

    if flags.install {
        step.next("Installing on device (adb)");
        let adb = tc.adb.as_ref().expect("checked above");
        run(Command::new(adb).arg("install").arg("-r").arg(&output), "adb install", v)?;
        run(
            Command::new(adb).args([
                "shell",
                "am",
                "start",
                "-n",
                &format!("{}/.MainActivity", cfg.package),
            ]),
            "adb shell am start",
            v,
        )?;
    }

    if let Some(dir) = &flags.keep_build {
        println!("Build files kept in {}", dir.display());
    }
    Ok(output)
}

fn javac(tc: &Toolchain, sources: &[PathBuf], out: &Path, verbose: bool) -> Result<()> {
    let base = |cmd: &mut Command| {
        cmd.args(["-encoding", "UTF-8", "-nowarn", "-g:none"]).arg("-d").arg(out);
    };
    // Java 8 bytecode against android.jar as the boot classpath is the most
    // faithful setup; JDKs that dropped `-source 8` fall back to 11.
    let mut cmd = Command::new(&tc.jdk.javac);
    base(&mut cmd);
    cmd.args(["-source", "8", "-target", "8", "-Xlint:-options", "-bootclasspath"])
        .arg(&tc.platform.android_jar)
        .args(sources);
    match run(&mut cmd, "javac", verbose) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("option 8") => {
            let mut cmd = Command::new(&tc.jdk.javac);
            base(&mut cmd);
            cmd.args(["-source", "11", "-target", "11", "-classpath"])
                .arg(&tc.platform.android_jar)
                .args(sources);
            run(&mut cmd, "javac", verbose).map(|_| ())
        }
        Err(e) => Err(e),
    }
}

/// Copies every entry of the aapt2 output untouched (resources.arsc must stay
/// stored/uncompressed) and appends the dex files.
fn add_dex(base_apk: &Path, dex_dir: &Path, out: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(base_apk)?)?;
    let mut writer = ZipWriter::new(File::create(out)?);
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        writer.raw_copy_file(entry)?;
    }
    let mut dexes = Vec::new();
    collect_files(dex_dir, "dex", &mut dexes)?;
    dexes.sort();
    if dexes.is_empty() {
        bail!("d8 produced no classes.dex");
    }
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for dex in dexes {
        let name = dex.file_name().unwrap().to_string_lossy().into_owned();
        writer.start_file(name, opts)?;
        let mut data = Vec::new();
        File::open(&dex)?.read_to_end(&mut data)?;
        writer.write_all(&data)?;
    }
    writer.finish()?;
    Ok(())
}

struct Signing {
    keystore: PathBuf,
    alias: String,
    store_password: String,
    key_password: String,
}

fn resolve_signing(cfg: &AppConfig, jdk: &Jdk, verbose: bool) -> Result<Signing> {
    if let Some(ks) = &cfg.keystore {
        if !ks.is_file() {
            bail!("keystore not found: {} (create one with `url-to-apk keygen`)", ks.display());
        }
        let store_password = cfg
            .store_password
            .clone()
            .context("--keystore needs a password: set URL2APK_STORE_PASSWORD or pass --store-password")?;
        let alias = cfg.key_alias.clone().context("--keystore needs --key-alias")?;
        return Ok(Signing {
            keystore: ks.clone(),
            alias,
            key_password: cfg.key_password.clone().unwrap_or_else(|| store_password.clone()),
            store_password,
        });
    }

    let dir = dirs::home_dir().context("cannot determine home directory")?.join(".url-to-apk");
    let keystore = dir.join("default.keystore");
    if !keystore.is_file() {
        std::fs::create_dir_all(&dir)?;
        println!("      Creating default signing key at {}", keystore.display());
        generate_keystore(
            jdk,
            &keystore,
            DEFAULT_KEY_ALIAS,
            DEFAULT_KEY_PASSWORD,
            "CN=url-to-apk, OU=Default, O=url-to-apk",
            10000,
            verbose,
        )?;
    }
    Ok(Signing {
        keystore,
        alias: DEFAULT_KEY_ALIAS.into(),
        store_password: DEFAULT_KEY_PASSWORD.into(),
        key_password: DEFAULT_KEY_PASSWORD.into(),
    })
}

/// Creates a PKCS12 keystore with a single RSA key (store and key share the password).
pub fn generate_keystore(
    jdk: &Jdk,
    path: &Path,
    alias: &str,
    password: &str,
    dname: &str,
    validity_days: u32,
    verbose: bool,
) -> Result<()> {
    if password.len() < 6 {
        bail!("keystore password must be at least 6 characters");
    }
    run(
        Command::new(&jdk.keytool)
            .args(["-genkeypair", "-noprompt", "-storetype", "PKCS12", "-keystore"])
            .arg(path)
            .args(["-alias", alias, "-keyalg", "RSA", "-keysize", "2048"])
            .args(["-validity", &validity_days.to_string()])
            .args(["-dname", dname])
            .args(["-storepass:env", STORE_PASS_ENV, "-keypass:env", STORE_PASS_ENV])
            .env(STORE_PASS_ENV, password),
        "keytool",
        verbose,
    )?;
    Ok(())
}

fn output_path(cfg: &AppConfig) -> PathBuf {
    if let Some(o) = &cfg.output {
        return o.clone();
    }
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    };
    let name = Some(safe(&cfg.name)).filter(|s| !s.is_empty()).unwrap_or_else(|| "app".into());
    PathBuf::from(format!("{name}-{}.apk", safe(&cfg.version_name)))
}

fn collect_files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, ext, out)?;
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
    Ok(())
}

/// Runs a tool, returning its combined output or an error that includes it.
pub fn run(cmd: &mut Command, what: &str, verbose: bool) -> Result<String> {
    if verbose {
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        println!("      $ {} {}", cmd.get_program().to_string_lossy(), args.join(" "));
    }
    let out = cmd
        .output()
        .with_context(|| format!("failed to start {what} ({})", cmd.get_program().to_string_lossy()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        bail!("{what} failed ({}):\n{}", out.status, text.trim());
    }
    if verbose && !text.trim().is_empty() {
        for line in text.trim().lines() {
            println!("        {line}");
        }
    }
    Ok(text)
}
