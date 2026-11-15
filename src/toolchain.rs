//! Locates the JDK and Android SDK pieces needed for a build.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Oldest platform we can compile against (OnBackInvokedCallback needs API 33).
pub const MIN_COMPILE_SDK: u32 = 33;

#[derive(Debug, Clone)]
pub struct Jdk {
    pub home: PathBuf,
    pub java: PathBuf,
    pub javac: PathBuf,
    pub keytool: PathBuf,
}

#[derive(Debug, Clone)]
pub struct BuildTools {
    pub version: String,
    pub aapt2: PathBuf,
    pub zipalign: PathBuf,
    pub d8_jar: PathBuf,
    pub apksigner_jar: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Platform {
    pub api: u32,
    pub android_jar: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Toolchain {
    pub jdk: Jdk,
    pub sdk: PathBuf,
    pub build_tools: BuildTools,
    pub platform: Platform,
    pub adb: Option<PathBuf>,
}

impl Toolchain {
    pub fn detect(sdk: Option<&Path>, jdk: Option<&Path>, compile_sdk: Option<u32>) -> Result<Self> {
        let jdk = find_jdk(jdk)?;
        let sdk = find_sdk(sdk)?;
        let build_tools = find_build_tools(&sdk)?;
        let platform = find_platform(&sdk, compile_sdk)?;
        let adb = Some(sdk.join("platform-tools").join(exe("adb"))).filter(|p| p.is_file());
        Ok(Toolchain { jdk, sdk, build_tools, platform, adb })
    }
}

pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn jdk_at(home: &Path) -> Option<Jdk> {
    let bin = home.join("bin");
    let jdk = Jdk {
        home: home.to_path_buf(),
        java: bin.join(exe("java")),
        javac: bin.join(exe("javac")),
        keytool: bin.join(exe("keytool")),
    };
    (jdk.java.is_file() && jdk.javac.is_file() && jdk.keytool.is_file()).then_some(jdk)
}

/// Asks the `java` on PATH where it lives.
fn java_home_from_path() -> Option<PathBuf> {
    let out = Command::new("java").args(["-XshowSettings:properties", "-version"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.trim().strip_prefix("java.home = "))
        .map(|h| PathBuf::from(h.trim()))
}

pub fn find_jdk(explicit: Option<&Path>) -> Result<Jdk> {
    if let Some(p) = explicit {
        return jdk_at(p).with_context(|| {
            format!("{} is not a JDK (expected bin/java, bin/javac and bin/keytool)", p.display())
        });
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(h) = std::env::var_os("JAVA_HOME") {
        candidates.push(h.into());
    }
    if let Some(h) = java_home_from_path() {
        candidates.push(h);
    }
    // JetBrains Runtime bundled with Android Studio.
    if cfg!(windows) {
        candidates.push(r"C:\Program Files\Android\Android Studio\jbr".into());
    } else if cfg!(target_os = "macos") {
        candidates.push("/Applications/Android Studio.app/Contents/jbr/Contents/Home".into());
    } else {
        candidates.push("/opt/android-studio/jbr".into());
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("android-studio/jbr"));
        }
    }
    candidates.iter().find_map(|c| jdk_at(c)).context(
        "no JDK found. Install JDK 17+ (e.g. https://adoptium.net) and set JAVA_HOME, or pass --jdk",
    )
}

pub fn find_sdk(explicit: Option<&Path>) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = explicit {
        candidates.push(p.into());
    } else {
        for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if let Some(v) = std::env::var_os(var) {
                candidates.push(v.into());
            }
        }
        if cfg!(windows) {
            if let Some(d) = dirs::data_local_dir() {
                candidates.push(d.join("Android").join("Sdk"));
            }
        } else if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Library/Android/sdk"));
            candidates.push(home.join("Android/Sdk"));
        }
    }
    let found = candidates
        .iter()
        .find(|c| c.join("build-tools").is_dir() || c.join("platforms").is_dir());
    match found {
        Some(p) => Ok(p.clone()),
        None if explicit.is_some() => bail!("{} is not an Android SDK directory", candidates[0].display()),
        None => bail!(
            "Android SDK not found. Install Android Studio or the command-line tools \
             (https://developer.android.com/studio#command-line-tools-only), then set ANDROID_HOME or pass --sdk"
        ),
    }
}

fn version_key(v: &str) -> Vec<u32> {
    v.split(['-', ' ']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn find_build_tools(sdk: &Path) -> Result<BuildTools> {
    let dir = sdk.join("build-tools");
    let mut found: Vec<BuildTools> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let bt = BuildTools {
                        version: e.file_name().to_string_lossy().into_owned(),
                        aapt2: p.join(exe("aapt2")),
                        zipalign: p.join(exe("zipalign")),
                        d8_jar: p.join("lib").join("d8.jar"),
                        apksigner_jar: p.join("lib").join("apksigner.jar"),
                    };
                    (bt.aapt2.is_file() && bt.zipalign.is_file() && bt.d8_jar.is_file() && bt.apksigner_jar.is_file())
                        .then_some(bt)
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|b| version_key(&b.version));
    found.pop().with_context(|| {
        format!(
            "no usable build-tools in {}. Install them with: sdkmanager \"build-tools;35.0.0\"",
            dir.display()
        )
    })
}

fn platform_api(dir: &Path) -> Option<u32> {
    let props = std::fs::read_to_string(dir.join("source.properties")).unwrap_or_default();
    props
        .lines()
        .find_map(|l| l.strip_prefix("AndroidVersion.ApiLevel="))
        .and_then(|v| v.trim().parse().ok())
        .or_else(|| {
            let name = dir.file_name()?.to_string_lossy().into_owned();
            name.strip_prefix("android-")?.split(['.', '-']).next()?.parse().ok()
        })
}

pub fn find_platform(sdk: &Path, wanted: Option<u32>) -> Result<Platform> {
    let dir = sdk.join("platforms");
    let mut found: Vec<Platform> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let jar = p.join("android.jar");
                    if !jar.is_file() {
                        return None;
                    }
                    Some(Platform { api: platform_api(&p)?, android_jar: jar })
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|p| p.api);

    if let Some(api) = wanted {
        if api < MIN_COMPILE_SDK {
            bail!("--compile-sdk must be at least {MIN_COMPILE_SDK}");
        }
        return found.into_iter().rev().find(|p| p.api == api).with_context(|| {
            format!("platform android-{api} is not installed. Install it with: sdkmanager \"platforms;android-{api}\"")
        });
    }
    match found.pop() {
        Some(p) if p.api >= MIN_COMPILE_SDK => Ok(p),
        _ => bail!(
            "no Android platform >= {MIN_COMPILE_SDK} found in {}. Install one with: sdkmanager \"platforms;android-35\"",
            dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::version_key;

    #[test]
    fn version_ordering() {
        assert!(version_key("36.0.0") > version_key("35.0.1"));
        assert!(version_key("34.0.0-rc3") > version_key("30.0.3"));
        assert!(version_key("30.0.10") > version_key("30.0.3"));
    }
}
