//! iOS support: generates a WKWebView app (Swift sources, Info.plist, asset
//! catalog) and builds it with the Xcode command-line tools:
//! swiftc -> actool -> codesign -> .ipa (devices) or .app (Simulator).

use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::builder::{self, BuildFlags, Step, run};
use crate::config::{AppConfig, Orientation, Permission};
use crate::icon;

const APP_SWIFT: &str = include_str!("templates/ios/App.swift");
/// CFBundleExecutable and Swift module name.
const EXECUTABLE: &str = "App";

/// Which SDK we compile against.
#[derive(Debug, Clone)]
pub struct Target {
    /// `iphoneos` or `iphonesimulator`
    pub sdk: &'static str,
    /// SDK version, e.g. "18.2"
    pub sdk_version: String,
    pub simulator: bool,
}

impl Target {
    fn supported_platform(&self) -> &'static str {
        if self.simulator {
            "iPhoneSimulator"
        } else {
            "iPhoneOS"
        }
    }
}

// ---------------------------------------------------------------- toolchain

struct Xcode {
    sdk_path: PathBuf,
    sdk_version: String,
}

fn xcrun(args: &[&str]) -> Result<String> {
    let out = Command::new("xcrun")
        .args(args)
        .output()
        .context("xcrun not found. Install Xcode from the App Store, then run `sudo xcode-select -s /Applications/Xcode.app`")?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        bail!(
            "`xcrun {}` failed: {}. iOS builds need the full Xcode app (not just the Command Line Tools); \
             select it with `sudo xcode-select -s /Applications/Xcode.app`",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(text)
}

fn detect_xcode(sdk: &str) -> Result<Xcode> {
    let sdk_path = PathBuf::from(xcrun(&["--sdk", sdk, "--show-sdk-path"])?);
    let sdk_version = xcrun(&["--sdk", sdk, "--show-sdk-version"])?;
    xcrun(&["--sdk", sdk, "--find", "swiftc"])?;
    xcrun(&["--sdk", sdk, "--find", "actool"])?;
    Ok(Xcode {
        sdk_path,
        sdk_version,
    })
}

fn ensure_macos() -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!(
            "iOS apps can only be built on macOS with Xcode (Apple's SDK and signing tools don't run on {}). \
             Run this command on a Mac, or on a macOS CI runner such as GitHub Actions `runs-on: macos-latest`",
            std::env::consts::OS
        );
    }
    Ok(())
}

/// Simulator architecture of this Mac (checks the hardware, so a Rosetta
/// build of this tool still targets arm64 on Apple silicon).
fn simulator_arch() -> &'static str {
    let arm = Command::new("sysctl")
        .args(["-n", "hw.optional.arm64"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(cfg!(target_arch = "aarch64"));
    if arm { "arm64" } else { "x86_64" }
}

pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// Environment checks for `doctor --platform ios`.
pub fn doctor_checks() -> Vec<(&'static str, CheckStatus, String)> {
    let mut checks = Vec::new();
    if let Err(e) = ensure_macos() {
        checks.push(("macOS", CheckStatus::Fail, format!("{e:#}")));
        return checks;
    }
    match xcrun(&["--find", "xcodebuild"]).and_then(|_| xcrun(&["xcodebuild", "-version"])) {
        Ok(v) => checks.push((
            "Xcode",
            CheckStatus::Ok,
            v.lines().next().unwrap_or_default().to_string(),
        )),
        Err(e) => checks.push(("Xcode", CheckStatus::Fail, format!("{e:#}"))),
    }
    for sdk in ["iphoneos", "iphonesimulator"] {
        let label = if sdk == "iphoneos" {
            "iOS SDK"
        } else {
            "Simulator"
        };
        match detect_xcode(sdk) {
            Ok(x) => checks.push((label, CheckStatus::Ok, format!("{sdk} {}", x.sdk_version))),
            Err(e) => checks.push((label, CheckStatus::Fail, format!("{e:#}"))),
        }
    }
    let identities = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let count = identities
        .lines()
        .filter(|l| l.trim_start().starts_with(|c: char| c.is_ascii_digit()) && l.contains('"'))
        .count();
    if count > 0 {
        checks.push((
            "signing",
            CheckStatus::Ok,
            format!(
                "{count} code signing identit{}",
                if count == 1 { "y" } else { "ies" }
            ),
        ));
    } else {
        checks.push((
            "signing",
            CheckStatus::Warn,
            "no code signing identity (only needed with --provisioning-profile; Simulator and ad-hoc builds work without)".into(),
        ));
    }
    checks
}

// ---------------------------------------------------------------- build

pub fn build(cfg: &AppConfig, flags: &BuildFlags) -> Result<PathBuf> {
    ensure_macos()?;
    if flags.install && !cfg.simulator {
        bail!(
            "--install is only supported with --simulator. For a device, install the .ipa with Xcode \
             (Window > Devices and Simulators), Apple Configurator, or `xcrun devicectl device install app --device <UDID> <ipa>`"
        );
    }
    if cfg.simulator && cfg.provisioning_profile.is_some() {
        println!("warning: --provisioning-profile is ignored for Simulator builds");
    }
    if cfg.deep_links {
        println!(
            "warning: deep_links is ignored on iOS (Universal Links need an Associated Domains entitlement)"
        );
    }

    let sdk = if cfg.simulator {
        "iphonesimulator"
    } else {
        "iphoneos"
    };
    let xcode = detect_xcode(sdk)?;
    let target = Target {
        sdk,
        sdk_version: xcode.sdk_version.clone(),
        simulator: cfg.simulator,
    };
    let arch = if cfg.simulator {
        simulator_arch()
    } else {
        "arm64"
    };
    let triple = format!(
        "{arch}-apple-ios{}{}",
        cfg.ios_min_version,
        if cfg.simulator { "-simulator" } else { "" }
    );

    println!(
        "Building {} ({}) for iOS{} from {}",
        cfg.name,
        cfg.bundle_id,
        if cfg.simulator { " Simulator" } else { "" },
        cfg.url
    );
    if flags.verbose {
        println!(
            "  SDK: {} ({sdk} {}) | target {triple}",
            xcode.sdk_path.display(),
            xcode.sdk_version
        );
    }

    let (ws, _temp) = builder::workspace(flags)?;
    let v = flags.verbose;
    let mut step = Step {
        n: 0,
        total: if flags.install { 6 } else { 5 },
    };

    step.next("Generating iOS project");
    let proj = generate(cfg, &ws.join("project"), &target)?;
    let app = ws.join(format!("{}.app", bundle_dir_name(cfg)));
    std::fs::create_dir_all(&app)?;

    step.next("Compiling Swift (swiftc)");
    let mut swiftc = Command::new("xcrun");
    swiftc
        .args([
            "--sdk",
            sdk,
            "swiftc",
            "-parse-as-library",
            "-swift-version",
            "5",
            "-O",
            "-whole-module-optimization",
        ])
        .args(["-target", &triple])
        .arg("-sdk")
        .arg(&xcode.sdk_path)
        .args(["-module-name", EXECUTABLE])
        .arg("-o")
        .arg(app.join(EXECUTABLE))
        .args(&proj.sources);
    run(&mut swiftc, "swiftc", v)?;

    step.next("Compiling assets (actool)");
    let partial = ws.join("assets-partial.plist");
    run(
        Command::new("xcrun")
            .args(["--sdk", sdk, "actool"])
            .arg(&proj.assets)
            .arg("--compile")
            .arg(&app)
            .args([
                "--platform",
                sdk,
                "--minimum-deployment-target",
                &cfg.ios_min_version,
            ])
            .args([
                "--app-icon",
                "AppIcon",
                "--target-device",
                "iphone",
                "--target-device",
                "ipad",
            ])
            .args([
                "--output-format",
                "human-readable-text",
                "--errors",
                "--warnings",
            ])
            .arg("--output-partial-info-plist")
            .arg(&partial),
        "actool",
        v,
    )?;
    let info_plist = app.join("Info.plist");
    std::fs::copy(&proj.info_plist, &info_plist)?;
    if partial.is_file() {
        // Adds CFBundleIcons etc. that point at the compiled Assets.car.
        run(
            Command::new("/usr/libexec/PlistBuddy")
                .arg("-c")
                .arg(format!("Merge {}", partial.display()))
                .arg(&info_plist),
            "PlistBuddy merge",
            v,
        )?;
    }
    std::fs::write(app.join("PkgInfo"), "APPL????")?;

    step.next("Signing (codesign)");
    let signed_with = sign(cfg, &app, &ws, v)?;

    let output = builder::output_path(cfg, if cfg.simulator { "app" } else { "ipa" });
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    if cfg.simulator {
        step.next("Copying app bundle");
        if output.exists() {
            if output.is_dir() && output.extension().is_some_and(|e| e == "app") {
                std::fs::remove_dir_all(&output)
                    .with_context(|| format!("cannot replace {}", output.display()))?;
            } else {
                bail!(
                    "{} already exists and is not an .app bundle; choose another --output",
                    output.display()
                );
            }
        }
        copy_dir(&app, &output)?;
    } else {
        step.next("Packaging IPA");
        write_ipa(&app, &output)?;
    }

    if flags.install {
        step.next("Installing on the booted Simulator (simctl)");
        run(
            Command::new("xcrun")
                .args(["simctl", "install", "booted"])
                .arg(&output),
            "simctl install",
            v,
        )
        .map_err(|e| {
            if e.to_string().contains("No devices are booted") {
                anyhow::anyhow!(
                    "no Simulator is running. Start one with `open -a Simulator`, then retry"
                )
            } else {
                e
            }
        })?;
        run(
            Command::new("xcrun").args(["simctl", "launch", "booted", &cfg.bundle_id]),
            "simctl launch",
            v,
        )?;
    }

    match signed_with {
        Signed::AdHoc if !cfg.simulator => println!(
            "      Ad-hoc signed: to run it on a device, re-sign with your Apple ID (e.g. Sideloadly, AltStore)\n      \
             or rebuild with --provisioning-profile"
        ),
        Signed::Identity(id) => println!("      Signed with \"{id}\""),
        _ => {}
    }
    if let Some(dir) = &flags.keep_build {
        println!("Build files kept in {}", dir.display());
    }
    Ok(output)
}

enum Signed {
    AdHoc,
    Identity(String),
}

fn sign(cfg: &AppConfig, app: &Path, ws: &Path, v: bool) -> Result<Signed> {
    let profile = match &cfg.provisioning_profile {
        Some(p) if !cfg.simulator => p,
        _ => {
            let identity = cfg.signing_identity.clone().filter(|_| !cfg.simulator);
            let id = identity.as_deref().unwrap_or("-");
            run(
                Command::new("codesign")
                    .args(["--force", "--sign", id, "--timestamp=none"])
                    .arg(app),
                "codesign",
                v,
            )?;
            return Ok(identity.map(Signed::Identity).unwrap_or(Signed::AdHoc));
        }
    };

    // Decode the profile and derive the entitlements from it.
    let decoded = ws.join("profile.plist");
    run(
        Command::new("security")
            .args(["cms", "-D", "-i"])
            .arg(profile)
            .arg("-o")
            .arg(&decoded),
        "security cms (decode provisioning profile)",
        v,
    )?;
    let buddy = |cmd: &str, file: &Path| -> Result<String> {
        Ok(run(
            Command::new("/usr/libexec/PlistBuddy")
                .arg("-c")
                .arg(cmd)
                .arg(file),
            "PlistBuddy",
            false,
        )?
        .trim()
        .to_string())
    };
    let app_id = buddy("Print :Entitlements:application-identifier", &decoded)
        .context("the provisioning profile has no application-identifier")?;
    let (team, pattern) = app_id
        .split_once('.')
        .context("unexpected application-identifier in profile")?;
    if !app_id_matches(pattern, &cfg.bundle_id) {
        bail!(
            "provisioning profile is for `{pattern}`, but the bundle ID is `{}` (set --bundle-id to match)",
            cfg.bundle_id
        );
    }
    let entitlements = ws.join("entitlements.plist");
    let entitlements_xml = run(
        Command::new("/usr/libexec/PlistBuddy")
            .args(["-x", "-c", "Print :Entitlements"])
            .arg(&decoded),
        "PlistBuddy (read entitlements)",
        false,
    )?;
    std::fs::write(&entitlements, entitlements_xml)?;
    buddy(
        &format!("Set :application-identifier {team}.{}", cfg.bundle_id),
        &entitlements,
    )?;
    let debuggable =
        buddy("Print :Entitlements:get-task-allow", &decoded).is_ok_and(|s| s == "true");
    let identity = cfg.signing_identity.clone().unwrap_or_else(|| {
        if debuggable {
            "Apple Development"
        } else {
            "Apple Distribution"
        }
        .to_string()
    });

    std::fs::copy(profile, app.join("embedded.mobileprovision"))?;
    run(
        Command::new("codesign")
            .args(["--force", "--sign", &identity, "--timestamp=none", "--generate-entitlement-der"])
            .arg("--entitlements")
            .arg(&entitlements)
            .arg(app),
        "codesign",
        v,
    )
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("no identity found") || msg.contains("ambiguous") {
            anyhow::anyhow!(
                "codesign could not pick the identity \"{identity}\": {}\n\
                 List yours with `security find-identity -v -p codesigning` and pass --signing-identity",
                msg.lines().last().unwrap_or_default()
            )
        } else {
            e
        }
    })?;
    Ok(Signed::Identity(identity))
}

/// `pattern` is the profile's app ID without the team prefix: exact, `*`, or `com.example.*`.
fn app_id_matches(pattern: &str, bundle_id: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => bundle_id.starts_with(prefix),
        None => pattern == bundle_id,
    }
}

fn bundle_dir_name(cfg: &AppConfig) -> String {
    let safe = builder::safe_file_name(&cfg.name);
    if safe.is_empty() {
        EXECUTABLE.to_string()
    } else {
        safe
    }
}

/// An IPA is a zip with the bundle under `Payload/`; the executable must keep its mode.
fn write_ipa(app: &Path, out: &Path) -> Result<()> {
    let mut zip = ZipWriter::new(
        File::create(out).with_context(|| format!("cannot create {}", out.display()))?,
    );
    let root = format!("Payload/{}", app.file_name().unwrap().to_string_lossy());
    zip.add_directory(
        "Payload/",
        SimpleFileOptions::default().unix_permissions(0o755),
    )?;
    add_dir_to_zip(&mut zip, app, &root, app)?;
    zip.finish()?;
    Ok(())
}

fn add_dir_to_zip(zip: &mut ZipWriter<File>, dir: &Path, prefix: &str, app: &Path) -> Result<()> {
    zip.add_directory(
        format!("{prefix}/"),
        SimpleFileOptions::default().unix_permissions(0o755),
    )?;
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        if path.is_dir() {
            add_dir_to_zip(zip, &path, &name, app)?;
        } else {
            let executable = path == app.join(EXECUTABLE);
            let opts = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(if executable { 0o755 } else { 0o644 });
            zip.start_file(name, opts)?;
            zip.write_all(&std::fs::read(&path)?)?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- project

pub struct IosProject {
    pub sources: Vec<PathBuf>,
    pub info_plist: PathBuf,
    pub assets: PathBuf,
}

pub fn generate(cfg: &AppConfig, dir: &Path, target: &Target) -> Result<IosProject> {
    std::fs::create_dir_all(dir)?;
    let app_swift = dir.join("App.swift");
    std::fs::write(&app_swift, APP_SWIFT)?;
    let config_swift = dir.join("Config.swift");
    std::fs::write(&config_swift, config_swift_source(cfg))?;

    let info_plist = dir.join("Info.plist");
    std::fs::write(&info_plist, info_plist_xml(cfg, target))?;

    let assets = dir.join("Assets.xcassets");
    let icon_set = assets.join("AppIcon.appiconset");
    let color_set = assets.join("LaunchBackground.colorset");
    std::fs::create_dir_all(&icon_set)?;
    std::fs::create_dir_all(&color_set)?;
    std::fs::write(
        assets.join("Contents.json"),
        r#"{ "info" : { "author" : "url2apk", "version" : 1 } }"#,
    )?;
    icon::write_ios_icon(
        &icon_set.join("AppIcon.png"),
        cfg.icon.as_deref(),
        cfg.theme_color,
        cfg.icon_background,
    )?;
    std::fs::write(
        icon_set.join("Contents.json"),
        r#"{
  "images" : [ { "filename" : "AppIcon.png", "idiom" : "universal", "platform" : "ios", "size" : "1024x1024" } ],
  "info" : { "author" : "url2apk", "version" : 1 }
}
"#,
    )?;
    std::fs::write(
        color_set.join("Contents.json"),
        color_set_json(cfg.background_color),
    )?;

    Ok(IosProject {
        sources: vec![app_swift, config_swift],
        info_plist,
        assets,
    })
}

fn color_set_json(argb: u32) -> String {
    let c = |shift: u32| format!("0x{:02X}", (argb >> shift) & 0xFF);
    format!(
        r#"{{
  "colors" : [ {{ "idiom" : "universal", "color" : {{ "color-space" : "srgb", "components" : {{ "red" : "{}", "green" : "{}", "blue" : "{}", "alpha" : "1.000" }} }} }} ],
  "info" : {{ "author" : "url2apk", "version" : 1 }}
}}
"#,
        c(16),
        c(8),
        c(0)
    )
}

fn is_light(argb: u32) -> bool {
    let (r, g, b) = ((argb >> 16) & 0xFF, (argb >> 8) & 0xFF, argb & 0xFF);
    (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) / 255.0 > 0.6
}

fn config_swift_source(cfg: &AppConfig) -> String {
    let hosts = cfg
        .allowed_hosts
        .iter()
        .map(|h| swift_string(h))
        .collect::<Vec<_>>()
        .join(", ");
    let orientations = match cfg.orientation {
        // Intersected with Info.plist, so iPhones still skip upside-down.
        Orientation::Auto | Orientation::Sensor => ".all",
        Orientation::Portrait => "[.portrait, .portraitUpsideDown]",
        Orientation::Landscape => ".landscape",
    };
    let status_bar_color = if cfg.fullscreen {
        cfg.background_color
    } else {
        cfg.theme_color
    };
    format!(
        r#"// Generated by url2apk. Do not edit.

import UIKit

enum Config {{
    static let startURL = URL(string: {url})!
    static let allowedHosts: [String] = [{hosts}]
    static let openExternalLinks = {external}
    static let javascript = {js}
    static let zoom = {zoom}
    static let downloads = {downloads}
    static let fullscreen = {fullscreen}
    static let keepScreenOn = {keep_on}
    static let pullToRefresh = {ptr}
    static let debugWebView = {debug}
    static let camera = {camera}
    static let microphone = {mic}
    static let userAgent = {ua}
    static let userAgentSuffix = {ua_suffix}
    static let themeColor = UIColor(argb: 0x{theme:08X})
    static let backgroundColor = UIColor(argb: 0x{bg:08X})
    static let progressColor = UIColor(argb: 0x{progress:08X})
    static let lightStatusBar = {light_status}
    static let lightBackground = {light_bg}
    static let lightTheme = {light_theme}
    static let orientations: UIInterfaceOrientationMask = {orientations}
    static let offlineTitle = {offline_title}
    static let offlineMessage = {offline_message}
    static let retryText = {retry}
}}
"#,
        url = swift_string(cfg.url.as_str()),
        external = cfg.open_external_links,
        js = cfg.javascript,
        zoom = cfg.zoom,
        downloads = cfg.downloads,
        fullscreen = cfg.fullscreen,
        keep_on = cfg.keep_screen_on,
        ptr = cfg.pull_to_refresh,
        debug = cfg.debug_webview,
        camera = cfg.has_permission(Permission::Camera),
        mic = cfg.has_permission(Permission::Microphone),
        ua = swift_string(&cfg.user_agent),
        ua_suffix = swift_string(&cfg.user_agent_suffix),
        theme = cfg.theme_color,
        bg = cfg.background_color,
        progress = cfg.progress_color,
        light_status = is_light(status_bar_color),
        light_bg = is_light(cfg.background_color),
        light_theme = is_light(cfg.theme_color),
        offline_title = swift_string(&cfg.offline_title),
        offline_message = swift_string(&cfg.offline_message),
        retry = swift_string(&cfg.retry_text),
    )
}

fn info_plist_xml(cfg: &AppConfig, target: &Target) -> String {
    use Plist::*;
    let s = |v: &str| Str(v.to_string());
    let strings = |v: &[&str]| Array(v.iter().map(|x| s(x)).collect());

    let (iphone, ipad): (&[&str], &[&str]) = match cfg.orientation {
        Orientation::Auto | Orientation::Sensor => (
            &[
                "UIInterfaceOrientationPortrait",
                "UIInterfaceOrientationLandscapeLeft",
                "UIInterfaceOrientationLandscapeRight",
            ],
            &[
                "UIInterfaceOrientationPortrait",
                "UIInterfaceOrientationPortraitUpsideDown",
                "UIInterfaceOrientationLandscapeLeft",
                "UIInterfaceOrientationLandscapeRight",
            ],
        ),
        Orientation::Portrait => (
            &["UIInterfaceOrientationPortrait"],
            &[
                "UIInterfaceOrientationPortrait",
                "UIInterfaceOrientationPortraitUpsideDown",
            ],
        ),
        Orientation::Landscape => (
            &[
                "UIInterfaceOrientationLandscapeLeft",
                "UIInterfaceOrientationLandscapeRight",
            ],
            &[
                "UIInterfaceOrientationLandscapeLeft",
                "UIInterfaceOrientationLandscapeRight",
            ],
        ),
    };

    let mut d: Vec<(&str, Plist)> = vec![
        ("CFBundleDevelopmentRegion", s("en")),
        ("CFBundleDisplayName", s(&cfg.name)),
        ("CFBundleName", s(&cfg.name)),
        ("CFBundleExecutable", s(EXECUTABLE)),
        ("CFBundleIdentifier", s(&cfg.bundle_id)),
        ("CFBundleInfoDictionaryVersion", s("6.0")),
        ("CFBundlePackageType", s("APPL")),
        ("CFBundleShortVersionString", s(&cfg.version_name)),
        ("CFBundleVersion", s(&cfg.version_code.to_string())),
        (
            "CFBundleSupportedPlatforms",
            strings(&[target.supported_platform()]),
        ),
        ("DTPlatformName", s(target.sdk)),
        ("DTPlatformVersion", s(&target.sdk_version)),
        (
            "DTSDKName",
            s(&format!("{}{}", target.sdk, target.sdk_version)),
        ),
        ("MinimumOSVersion", s(&cfg.ios_min_version)),
        ("LSRequiresIPhoneOS", Bool(true)),
        ("UIDeviceFamily", Array(vec![Int(1), Int(2)])),
        ("UIRequiredDeviceCapabilities", strings(&["arm64"])),
        (
            "UILaunchScreen",
            Dict(vec![("UIColorName".into(), s("LaunchBackground"))]),
        ),
        (
            "UIApplicationSceneManifest",
            Dict(vec![
                ("UIApplicationSupportsMultipleScenes".into(), Bool(false)),
                (
                    "UISceneConfigurations".into(),
                    Dict(vec![(
                        "UIWindowSceneSessionRoleApplication".into(),
                        Array(vec![Dict(vec![
                            ("UISceneConfigurationName".into(), s("Default")),
                            ("UISceneDelegateClassName".into(), s("SceneDelegate")),
                        ])]),
                    )]),
                ),
            ]),
        ),
        ("UISupportedInterfaceOrientations", strings(iphone)),
        ("UISupportedInterfaceOrientations~ipad", strings(ipad)),
        ("ITSAppUsesNonExemptEncryption", Bool(false)),
    ];
    if matches!(
        cfg.orientation,
        Orientation::Portrait | Orientation::Landscape
    ) {
        // iPad multitasking requires every orientation.
        d.push(("UIRequiresFullScreen", Bool(true)));
    }
    if cfg.fullscreen {
        d.push(("UIStatusBarHidden", Bool(true)));
    }
    if cfg.allow_http {
        d.push((
            "NSAppTransportSecurity",
            Dict(vec![("NSAllowsArbitraryLoads".into(), Bool(true))]),
        ));
    }
    // Always declared: <input type=file> offers "Take Photo/Video", and iOS
    // terminates apps that touch the camera/mic without a usage description.
    let name = &cfg.name;
    d.push((
        "NSCameraUsageDescription",
        s(&format!(
            "{name} uses the camera when a web page asks for it, e.g. to take a photo to upload."
        )),
    ));
    d.push(("NSMicrophoneUsageDescription", s(&format!("{name} uses the microphone when a web page asks for it, e.g. for calls or recording video."))));
    if cfg.downloads {
        d.push((
            "NSPhotoLibraryAddUsageDescription",
            s(&format!(
                "{name} saves downloaded images to your photo library when you ask it to."
            )),
        ));
    }
    if cfg.has_permission(Permission::Location) {
        d.push((
            "NSLocationWhenInUseUsageDescription",
            s(&format!(
                "{name} shares your location with the website when it asks for it."
            )),
        ));
    }

    let dict = Dict(d.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n",
    );
    dict.write(&mut out, 0);
    out.push_str("</plist>\n");
    out
}

enum Plist {
    Str(String),
    Bool(bool),
    Int(i64),
    Array(Vec<Plist>),
    Dict(Vec<(String, Plist)>),
}

impl Plist {
    fn write(&self, out: &mut String, depth: usize) {
        let pad = "\t".repeat(depth);
        match self {
            Plist::Str(s) => {
                let _ = writeln!(out, "{pad}<string>{}</string>", xml_escape(s));
            }
            Plist::Bool(b) => {
                let _ = writeln!(out, "{pad}<{b}/>");
            }
            Plist::Int(i) => {
                let _ = writeln!(out, "{pad}<integer>{i}</integer>");
            }
            Plist::Array(items) => {
                let _ = writeln!(out, "{pad}<array>");
                for item in items {
                    item.write(out, depth + 1);
                }
                let _ = writeln!(out, "{pad}</array>");
            }
            Plist::Dict(entries) => {
                let _ = writeln!(out, "{pad}<dict>");
                for (k, v) in entries {
                    let _ = writeln!(out, "{pad}\t<key>{}</key>", xml_escape(k));
                    v.write(out, depth + 1);
                }
                let _ = writeln!(out, "{pad}</dict>");
            }
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Swift string literal (escapes `\`, so `\(` can't start an interpolation).
fn swift_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                let _ = write!(out, "\\u{{{:X}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{self, AppOptions};

    fn cfg(extra: impl FnOnce(&mut AppOptions)) -> AppConfig {
        let mut o = AppOptions {
            url: Some("https://www.example.com/app".into()),
            ..Default::default()
        };
        extra(&mut o);
        config::resolve(o).unwrap()
    }

    fn target() -> Target {
        Target {
            sdk: "iphoneos",
            sdk_version: "18.2".into(),
            simulator: false,
        }
    }

    #[test]
    fn swift_escaping() {
        assert_eq!(swift_string(r#"a"b\(c)"#), r#""a\"b\\(c)""#);
        assert_eq!(swift_string("x\ny\u{1}"), r#""x\ny\u{1}""#);
    }

    #[test]
    fn app_ids() {
        assert!(app_id_matches("com.example.app", "com.example.app"));
        assert!(app_id_matches("*", "com.example.app"));
        assert!(app_id_matches("com.example.*", "com.example.app"));
        assert!(!app_id_matches("com.other.*", "com.example.app"));
        assert!(!app_id_matches("com.example.app2", "com.example.app"));
    }

    #[test]
    fn info_plist() {
        let c = cfg(|o| {
            o.name = Some("Tom & Jerry's <App>".into());
            o.orientation = Some(Orientation::Portrait);
            o.permissions = vec![Permission::Location];
        });
        let xml = info_plist_xml(&c, &target());
        assert!(xml.contains("<string>Tom &amp; Jerry's &lt;App&gt;</string>"));
        assert!(xml.contains("<key>CFBundleIdentifier</key>\n\t<string>com.example</string>"));
        assert!(xml.contains("<key>DTSDKName</key>\n\t<string>iphoneos18.2</string>"));
        assert!(xml.contains("<key>UIRequiresFullScreen</key>\n\t<true/>"));
        assert!(xml.contains("NSLocationWhenInUseUsageDescription"));
        assert!(!xml.contains("NSAppTransportSecurity"));
        assert_eq!(
            xml.matches("<dict>").count(),
            xml.matches("</dict>").count()
        );
    }

    #[test]
    fn bundle_id_default() {
        let c = cfg(|o| o.url = Some("https://my_site.example.org".into()));
        assert_eq!(c.package, "org.example.my_site");
        assert_eq!(c.bundle_id, "org.example.my-site");
    }

    #[test]
    fn generates_project() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(|o| o.theme_color = Some("#102030".into()));
        let proj = generate(&c, dir.path(), &target()).unwrap();
        assert_eq!(proj.sources.len(), 2);
        let config = std::fs::read_to_string(&proj.sources[1]).unwrap();
        assert!(
            config.contains("static let startURL = URL(string: \"https://www.example.com/app\")!")
        );
        assert!(config.contains("static let allowedHosts: [String] = [\"example.com\"]"));
        assert!(config.contains("UIColor(argb: 0xFF102030)"));
        let icon = image::open(proj.assets.join("AppIcon.appiconset/AppIcon.png")).unwrap();
        assert_eq!((icon.width(), icon.height()), (1024, 1024));
        assert!(!icon.color().has_alpha());
        let color =
            std::fs::read_to_string(proj.assets.join("LaunchBackground.colorset/Contents.json"))
                .unwrap();
        assert!(color.contains("\"red\" : \"0xFF\""));
    }
}
