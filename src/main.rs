mod builder;
mod config;
mod icon;
mod ios;
mod project;
mod toolchain;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use config::{AppOptions, DEFAULT_CONFIG_FILE, Platform};

#[derive(Parser)]
#[command(
    name = "url2apk",
    version,
    about = "Turn any website into an Android app (WebView APK) or an iOS app (IPA)",
    after_help = "Examples:\n  \
        url2apk build https://example.com\n  \
        url2apk build https://example.com -n \"Example\" -i logo.png --theme-color \"#0F9D58\" --pull-to-refresh\n  \
        url2apk build https://example.com --platform ios --simulator --install\n  \
        url2apk init https://example.com && url2apk build\n  \
        url2apk doctor"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build a signed APK (or an iOS app with --platform ios) for a website
    Build(Box<BuildArgs>),
    /// Write a url2apk.toml config file with every option documented
    Init {
        /// Website URL to put in the config
        url: Option<String>,
        /// Where to write the config
        #[arg(short, long, default_value = DEFAULT_CONFIG_FILE)]
        output: PathBuf,
        /// Overwrite existing files
        #[arg(short, long)]
        force: bool,
        /// Also write a CI workflow that builds the app (an existing config is then kept)
        #[arg(long, value_enum, value_name = "PROVIDER")]
        ci: Option<CiProvider>,
    },
    /// Check that the build tools are installed correctly
    Doctor {
        /// Platform to check [default: android]
        #[arg(long, value_enum)]
        platform: Option<Platform>,
        #[arg(long)]
        sdk: Option<PathBuf>,
        #[arg(long)]
        jdk: Option<PathBuf>,
    },
    /// Create a release keystore for signing your apps
    Keygen(KeygenArgs),
    /// Print version information
    Version,
}

#[derive(Args)]
struct BuildArgs {
    #[command(flatten)]
    app: AppOptions,

    /// Config file [default: ./url2apk.toml if it exists]
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Install and launch the app: on a connected device via adb (Android) or the booted Simulator (iOS)
    #[arg(long)]
    install: bool,

    /// Keep the generated project and intermediate files in this directory
    #[arg(long, value_name = "DIR")]
    keep_build: Option<PathBuf>,

    /// Print tool invocations and their output
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Args)]
struct KeygenArgs {
    /// Keystore file to create
    #[arg(short, long, default_value = "release.keystore")]
    output: PathBuf,
    /// Key alias
    #[arg(short, long, default_value = "release")]
    alias: String,
    /// Keystore/key password, at least 6 characters
    #[arg(long, env = "URL2APK_STORE_PASSWORD", hide_env_values = true)]
    password: String,
    /// Certificate subject
    #[arg(long, default_value = "CN=Android App")]
    dname: String,
    /// Validity in days
    #[arg(long, default_value_t = 10000)]
    validity: u32,
    #[arg(long)]
    jdk: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Cmd::Build(args) => cmd_build(*args),
        Cmd::Init {
            url,
            output,
            force,
            ci,
        } => cmd_init(url, &output, force, ci),
        Cmd::Doctor {
            platform: Some(Platform::Ios),
            ..
        } => cmd_doctor_ios(),
        Cmd::Doctor { sdk, jdk, .. } => cmd_doctor(sdk.as_deref(), jdk.as_deref()),
        Cmd::Keygen(args) => cmd_keygen(args),
        Cmd::Version => {
            println!("{}", version_string());
            Ok(())
        }
    }
}

fn version_string() -> String {
    format!(
        "{} {} ({}-{})",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn cmd_build(args: BuildArgs) -> Result<()> {
    let config_path = match args.config {
        Some(p) => Some(p),
        None => Some(PathBuf::from(DEFAULT_CONFIG_FILE)).filter(|p| p.is_file()),
    };
    let opts = match &config_path {
        Some(path) => {
            println!("Using config {}", path.display());
            args.app.or(AppOptions::from_file(path)?)
        }
        None => args.app,
    };
    let cfg = config::resolve(opts)?;
    let flags = builder::BuildFlags {
        keep_build: args.keep_build,
        verbose: args.verbose,
        install: args.install,
    };
    if cfg.platform == Platform::Ios {
        return finish_ios_build(&cfg, &flags);
    }
    let apk = builder::build(&cfg, &flags)?;

    let size = std::fs::metadata(&apk).map(|m| m.len()).unwrap_or(0);
    println!();
    println!("Done! {} ({:.1} KB)", apk.display(), size as f64 / 1024.0);
    println!(
        "  App:     {} {} (code {})",
        cfg.name, cfg.version_name, cfg.version_code
    );
    println!("  Package: {}", cfg.package);
    if cfg.keystore.is_none() {
        println!(
            "  Signed with the default key in ~/.url2apk. Keep it to ship updates, or use\n  \
             `url2apk keygen` + --keystore for a dedicated release key."
        );
    }
    if !args.install {
        println!("  Install: adb install -r \"{}\"", apk.display());
    }
    Ok(())
}

fn finish_ios_build(cfg: &config::AppConfig, flags: &builder::BuildFlags) -> Result<()> {
    let out = ios::build(cfg, flags)?;
    println!();
    if out.is_dir() {
        println!("Done! {}", out.display());
    } else {
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        println!("Done! {} ({:.1} KB)", out.display(), size as f64 / 1024.0);
    }
    println!(
        "  App:       {} {} (build {})",
        cfg.name, cfg.version_name, cfg.version_code
    );
    println!("  Bundle ID: {}", cfg.bundle_id);
    if cfg.simulator && !flags.install {
        println!(
            "  Install:   xcrun simctl install booted \"{}\"",
            out.display()
        );
    }
    Ok(())
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum CiProvider {
    /// GitHub Actions: .github/workflows/url2apk.yml
    Github,
}

/// The example workflow doubles as the `init --ci github` template.
const GITHUB_WORKFLOW: &str = include_str!("../examples/github-actions.yml");
const GITHUB_WORKFLOW_PATH: &str = ".github/workflows/url2apk.yml";

fn github_workflow(config_path: &Path) -> String {
    let config = config_path.to_string_lossy().replace('\\', "/");
    let config = config.strip_prefix("./").unwrap_or(&config);
    GITHUB_WORKFLOW.replace(
        &format!("URL2APK_CONFIG: {DEFAULT_CONFIG_FILE}"),
        &format!("URL2APK_CONFIG: {config}"),
    )
}

fn cmd_init(url: Option<String>, output: &Path, force: bool, ci: Option<CiProvider>) -> Result<()> {
    let workflow = ci.map(|CiProvider::Github| PathBuf::from(GITHUB_WORKFLOW_PATH));
    if let Some(wf) = workflow.as_ref().filter(|wf| wf.exists() && !force) {
        bail!("{} already exists (use --force to overwrite)", wf.display());
    }

    // With --ci, an existing config is reused so the command can add CI to a set-up project.
    let keep_config = output.exists() && !force && workflow.is_some();
    if output.exists() && !force && !keep_config {
        bail!(
            "{} already exists (use --force to overwrite)",
            output.display()
        );
    }
    if keep_config {
        println!("Keeping existing {}.", output.display());
    } else {
        let url = match url {
            Some(u) => config::parse_url(&u)?.to_string(),
            None => "https://example.com".to_string(),
        };
        if let Some(dir) = output.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(output, config::init_template(&url))
            .with_context(|| format!("cannot write {}", output.display()))?;
        println!(
            "Wrote {}. Edit it, then run `url2apk build`.",
            output.display()
        );
    }

    match workflow {
        Some(wf) => {
            if let Some(dir) = wf.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("cannot create {}", dir.display()))?;
            }
            std::fs::write(&wf, github_workflow(output))
                .with_context(|| format!("cannot write {}", wf.display()))?;
            println!(
                "Wrote {}. Commit both files, add the signing secrets listed at the top of the\n\
                 workflow, then push a tag (e.g. v1.0.0) or run it from the Actions tab.",
                wf.display()
            );
        }
        None => println!(
            "Tip: `url2apk init --ci github` adds a GitHub Actions workflow that builds the app."
        ),
    }
    Ok(())
}

fn cmd_doctor(sdk: Option<&Path>, jdk: Option<&Path>) -> Result<()> {
    let mut ok = true;
    let mut report = |label: &str, r: Result<String>| match r {
        Ok(s) => println!("  [ok]   {label:<12} {s}"),
        Err(e) => {
            ok = false;
            println!("  [fail] {label:<12} {e:#}");
        }
    };
    println!("Checking build environment:");
    report(
        "JDK",
        toolchain::find_jdk(jdk).map(|j| j.home.display().to_string()),
    );
    match toolchain::find_sdk(sdk) {
        Ok(root) => {
            report("Android SDK", Ok(root.display().to_string()));
            report(
                "build-tools",
                toolchain::find_build_tools(&root).map(|b| b.version),
            );
            report(
                "platform",
                toolchain::find_platform(&root, None).map(|p| format!("android-{}", p.api)),
            );
            let adb = root.join("platform-tools").join(toolchain::exe("adb"));
            if adb.is_file() {
                println!("  [ok]   {:<12} {}", "adb", adb.display());
            } else {
                println!(
                    "  [warn] {:<12} not installed (only needed for --install)",
                    "adb"
                );
            }
        }
        Err(e) => report("Android SDK", Err(e)),
    }
    if ok {
        println!("\nAll good. Try: url2apk build https://example.com");
        Ok(())
    } else {
        bail!("environment is incomplete; see the messages above")
    }
}

fn cmd_doctor_ios() -> Result<()> {
    println!("Checking iOS build environment:");
    let mut ok = true;
    for (label, status, msg) in ios::doctor_checks() {
        let tag = match status {
            ios::CheckStatus::Ok => "[ok]  ",
            ios::CheckStatus::Warn => "[warn]",
            ios::CheckStatus::Fail => {
                ok = false;
                "[fail]"
            }
        };
        println!("  {tag} {label:<12} {msg}");
    }
    if ok {
        println!(
            "\nAll good. Try: url2apk build https://example.com --platform ios --simulator"
        );
        Ok(())
    } else {
        bail!("environment is incomplete; see the messages above")
    }
}

fn cmd_keygen(args: KeygenArgs) -> Result<()> {
    if args.output.exists() {
        bail!(
            "{} already exists; refusing to overwrite a signing key",
            args.output.display()
        );
    }
    let jdk = toolchain::find_jdk(args.jdk.as_deref())?;
    builder::generate_keystore(
        &jdk,
        &args.output,
        &args.alias,
        &args.password,
        &args.dname,
        args.validity,
        false,
    )?;
    println!(
        "Created {} (alias `{}`).",
        args.output.display(),
        args.alias
    );
    println!("Back it up: you need this exact key to publish updates to your app.");
    println!(
        "Build with: url2apk build <URL> --keystore {} --key-alias {}  (password via URL2APK_STORE_PASSWORD)",
        args.output.display(),
        args.alias
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_template() {
        assert_eq!(github_workflow(Path::new("url2apk.toml")), GITHUB_WORKFLOW);
        let wf = github_workflow(Path::new(".\\apps\\shop.toml"));
        assert!(wf.contains("URL2APK_CONFIG: apps/shop.toml"));
        assert!(!wf.contains("URL2APK_CONFIG: url2apk.toml"));
        assert!(!wf.contains('\t'), "tabs are invalid YAML indentation");
    }

    #[test]
    fn version_command() {
        let cli = Cli::try_parse_from(["url2apk", "version"]).unwrap();
        assert!(matches!(cli.command, Cmd::Version));
        assert!(version_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn workflow_flags_exist() {
        // Every flag the workflow passes to `build` must exist.
        Cli::try_parse_from([
            "url2apk",
            "build",
            "--config",
            "x.toml",
            "--platform",
            "ios",
            "--simulator=false",
            "--version-code",
            "7",
            "-o",
            "dist/app.ipa",
            "--provisioning-profile",
            "p",
            "--keystore",
            "k",
            "--key-alias",
            "a",
        ])
        .map_err(|e| e.to_string())
        .unwrap();
    }
}
