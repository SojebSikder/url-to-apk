//! App options (shared by the CLI and the `url2apk.toml` config file) and
//! their resolution into a validated [`AppConfig`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde::Deserialize;
use url::Url;

pub const DEFAULT_CONFIG_FILE: &str = "url2apk.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Signed APK, built with the Android SDK (any OS)
    Android,
    /// IPA for devices or .app for the Simulator, built with Xcode (macOS only)
    Ios,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    /// Follow the device's auto-rotate setting
    Auto,
    Portrait,
    Landscape,
    /// Rotate freely using the sensor, ignoring the auto-rotate setting
    Sensor,
}

impl Orientation {
    pub fn manifest_value(self) -> &'static str {
        match self {
            Orientation::Auto => "unspecified",
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
            Orientation::Sensor => "fullSensor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    /// Geolocation API (navigator.geolocation)
    Location,
    /// Camera for getUserMedia and photo capture in file uploads
    Camera,
    /// Microphone for getUserMedia
    Microphone,
}

/// Every option can be given on the command line or in `url2apk.toml`
/// (same names, using snake_case in the file). CLI values win.
#[derive(Debug, Clone, Default, Args, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppOptions {
    /// Website URL to wrap, e.g. https://example.com
    pub url: Option<String>,

    /// Target platform [default: android]
    #[arg(long, value_enum)]
    pub platform: Option<Platform>,

    /// App name shown under the launcher icon [default: derived from the domain]
    #[arg(short, long, help_heading = "App")]
    pub name: Option<String>,

    /// Application ID / Java package, e.g. com.example.app [default: derived from the domain]
    #[arg(short = 'p', long, help_heading = "App")]
    pub package: Option<String>,

    /// Human-readable version [default: 1.0.0]
    #[arg(long, help_heading = "App")]
    pub version_name: Option<String>,

    /// Integer version; must increase for every update you publish [default: 1]
    #[arg(long, help_heading = "App")]
    pub version_code: Option<u32>,

    /// Launcher icon (PNG, JPEG or WebP, ideally 512x512) [default: generated]
    #[arg(short, long, help_heading = "Appearance")]
    pub icon: Option<PathBuf>,

    /// Background color of the adaptive icon, e.g. "#FFFFFF" [default: auto]
    #[arg(long, help_heading = "Appearance")]
    pub icon_background: Option<String>,

    /// Generate an Android 8+ adaptive icon [default: true]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Appearance")]
    pub adaptive_icon: Option<bool>,

    /// Status bar / accent color, e.g. "#2196F3"
    #[arg(long, help_heading = "Appearance")]
    pub theme_color: Option<String>,

    /// Page / splash screen background color [default: #FFFFFF]
    #[arg(long, help_heading = "Appearance")]
    pub background_color: Option<String>,

    /// Loading bar and spinner color [default: theme color]
    #[arg(long, help_heading = "Appearance")]
    pub progress_color: Option<String>,

    /// Screen orientation [default: auto]
    #[arg(long, value_enum, help_heading = "Appearance")]
    pub orientation: Option<Orientation>,

    /// Hide the status and navigation bars (immersive mode)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Appearance")]
    pub fullscreen: Option<bool>,

    /// Prevent the screen from turning off while the app is open
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Appearance")]
    pub keep_screen_on: Option<bool>,

    /// Enable pull-to-refresh
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub pull_to_refresh: Option<bool>,

    /// Open links to this website (from other apps) in the app
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub deep_links: Option<bool>,

    /// Enable JavaScript [default: true]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub javascript: Option<bool>,

    /// Allow pinch-to-zoom [default: false]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub zoom: Option<bool>,

    /// Handle file downloads with the system download manager [default: true]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub downloads: Option<bool>,

    /// Open links to other websites in the external browser [default: true]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub open_external_links: Option<bool>,

    /// Extra domain that stays inside the app (repeatable; subdomains included; "*" = all)
    #[arg(long = "allow-host", value_name = "HOST", help_heading = "Behavior")]
    pub allowed_hosts: Vec<String>,

    /// Replace the WebView user agent entirely
    #[arg(long, help_heading = "Behavior")]
    pub user_agent: Option<String>,

    /// Append text to the default user agent (lets your site detect the app)
    #[arg(long, help_heading = "Behavior")]
    pub user_agent_suffix: Option<String>,

    /// Allow plain-http (cleartext) traffic [default: true only for http:// URLs]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub allow_http: Option<bool>,

    /// Allow inspecting the WebView via chrome://inspect
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "Behavior")]
    pub debug_webview: Option<bool>,

    /// Runtime permission the website may request (repeatable)
    #[arg(
        long = "permission",
        value_enum,
        value_name = "PERMISSION",
        help_heading = "Behavior"
    )]
    pub permissions: Vec<Permission>,

    /// Title of the offline / error screen
    #[arg(long, help_heading = "Text")]
    pub offline_title: Option<String>,

    /// Message of the offline / error screen
    #[arg(long, help_heading = "Text")]
    pub offline_message: Option<String>,

    /// Label of the retry button
    #[arg(long, help_heading = "Text")]
    pub retry_text: Option<String>,

    /// Minimum Android API level (21 = Android 5.0) [default: 21]
    #[arg(long, help_heading = "Build")]
    pub min_sdk: Option<u32>,

    /// Target API level [default: the compile SDK]
    #[arg(long, help_heading = "Build")]
    pub target_sdk: Option<u32>,

    /// Android platform to compile against [default: newest installed]
    #[arg(long, help_heading = "Build")]
    pub compile_sdk: Option<u32>,

    /// Output APK path [default: ./<Name>-<version>.apk]
    #[arg(short, long, help_heading = "Build")]
    pub output: Option<PathBuf>,

    /// Android SDK directory [default: $ANDROID_HOME, $ANDROID_SDK_ROOT or the standard location]
    #[arg(long, help_heading = "Build")]
    pub sdk: Option<PathBuf>,

    /// JDK directory [default: $JAVA_HOME or the `java` on PATH]
    #[arg(long, help_heading = "Build")]
    pub jdk: Option<PathBuf>,

    /// Keystore used to sign the APK [default: auto-generated ~/.url-to-apk/default.keystore]
    #[arg(long, help_heading = "Signing")]
    pub keystore: Option<PathBuf>,

    /// Key alias inside the keystore
    #[arg(long, help_heading = "Signing")]
    pub key_alias: Option<String>,

    /// Keystore password (prefer the env var over the flag)
    #[arg(
        long,
        env = "URL2APK_STORE_PASSWORD",
        hide_env_values = true,
        help_heading = "Signing"
    )]
    pub store_password: Option<String>,

    /// Key password [default: the keystore password]
    #[arg(
        long,
        env = "URL2APK_KEY_PASSWORD",
        hide_env_values = true,
        help_heading = "Signing"
    )]
    pub key_password: Option<String>,

    /// iOS bundle identifier [default: the package, with `_` replaced by `-`]
    #[arg(long, help_heading = "iOS")]
    pub bundle_id: Option<String>,

    /// Minimum iOS version [default: 15.0]
    #[arg(long, value_name = "VERSION", help_heading = "iOS")]
    pub ios_min_version: Option<String>,

    /// Build a .app for the iOS Simulator instead of an .ipa for devices
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true", help_heading = "iOS")]
    pub simulator: Option<bool>,

    /// Provisioning profile (.mobileprovision) for signing device builds [default: ad-hoc signed]
    #[arg(long, value_name = "FILE", help_heading = "iOS")]
    pub provisioning_profile: Option<PathBuf>,

    /// Code signing identity, e.g. "Apple Development: Jane Doe (ABCDE12345)" [default: from the profile type]
    #[arg(long, value_name = "IDENTITY", help_heading = "iOS")]
    pub signing_identity: Option<String>,
}

impl AppOptions {
    /// Load options from a TOML file. Relative paths inside the file are
    /// resolved against the file's directory.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config file {}", path.display()))?;
        let mut opts: AppOptions = toml::from_str(&text)
            .with_context(|| format!("invalid config file {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        for p in [
            &mut opts.icon,
            &mut opts.output,
            &mut opts.keystore,
            &mut opts.sdk,
            &mut opts.jdk,
            &mut opts.provisioning_profile,
        ]
        .into_iter()
        .flatten()
        {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
        Ok(opts)
    }

    /// Fill every unset field of `self` from `fallback`.
    pub fn or(self, fallback: AppOptions) -> AppOptions {
        macro_rules! merge {
            (opt: $($o:ident),* ; vec: $($v:ident),*) => {
                AppOptions {
                    $($o: self.$o.or(fallback.$o),)*
                    $($v: if self.$v.is_empty() { fallback.$v } else { self.$v },)*
                }
            };
        }
        merge!(
            opt: url, platform, name, package, version_name, version_code, icon, icon_background, adaptive_icon,
                 theme_color, background_color, progress_color, orientation, fullscreen, keep_screen_on,
                 pull_to_refresh, deep_links, javascript, zoom, downloads, open_external_links,
                 user_agent, user_agent_suffix, allow_http, debug_webview, offline_title,
                 offline_message, retry_text, min_sdk, target_sdk, compile_sdk, output, sdk, jdk,
                 keystore, key_alias, store_password, key_password, bundle_id, ios_min_version,
                 simulator, provisioning_profile, signing_identity ;
            vec: allowed_hosts, permissions
        )
    }
}

/// Fully resolved and validated configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub platform: Platform,
    pub url: Url,
    pub name: String,
    pub package: String,
    pub version_name: String,
    pub version_code: u32,
    pub icon: Option<PathBuf>,
    pub icon_background: Option<u32>,
    pub adaptive_icon: bool,
    pub theme_color: u32,
    pub background_color: u32,
    pub progress_color: u32,
    pub orientation: Orientation,
    pub fullscreen: bool,
    pub keep_screen_on: bool,
    pub pull_to_refresh: bool,
    pub deep_links: bool,
    pub javascript: bool,
    pub zoom: bool,
    pub downloads: bool,
    pub open_external_links: bool,
    pub allowed_hosts: Vec<String>,
    pub user_agent: String,
    pub user_agent_suffix: String,
    pub allow_http: bool,
    pub debug_webview: bool,
    pub permissions: Vec<Permission>,
    pub offline_title: String,
    pub offline_message: String,
    pub retry_text: String,
    pub min_sdk: u32,
    pub target_sdk: Option<u32>,
    pub compile_sdk: Option<u32>,
    pub output: Option<PathBuf>,
    pub sdk: Option<PathBuf>,
    pub jdk: Option<PathBuf>,
    pub keystore: Option<PathBuf>,
    pub key_alias: Option<String>,
    pub store_password: Option<String>,
    pub key_password: Option<String>,
    pub bundle_id: String,
    pub ios_min_version: String,
    pub simulator: bool,
    pub provisioning_profile: Option<PathBuf>,
    pub signing_identity: Option<String>,
}

impl AppConfig {
    pub fn has_permission(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }

    /// Host used for default name/package/allowed-host derivation (no "www.").
    pub fn base_host(&self) -> String {
        base_host(&self.url)
    }
}

fn base_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or("app").to_ascii_lowercase();
    host.strip_prefix("www.")
        .map(str::to_string)
        .unwrap_or(host)
}

pub fn resolve(o: AppOptions) -> Result<AppConfig> {
    let raw_url = o
        .url
        .context("no website URL given (pass it as an argument or set `url` in url2apk.toml)")?;
    let url = parse_url(&raw_url)?;
    let host = base_host(&url);

    let name = match o.name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        Some(_) => bail!("--name must not be empty"),
        None => default_name(&host),
    };

    let package = match o.package {
        Some(p) => {
            validate_package(&p)?;
            p
        }
        None => default_package(&host),
    };

    let version_name = o.version_name.unwrap_or_else(|| "1.0.0".into());
    if version_name.trim().is_empty() {
        bail!("--version-name must not be empty");
    }
    let version_code = o.version_code.unwrap_or(1);
    if version_code == 0 || version_code > 2_100_000_000 {
        bail!("--version-code must be between 1 and 2100000000");
    }

    if let Some(icon) = &o.icon
        && !icon.is_file()
    {
        bail!("icon file not found: {}", icon.display());
    }

    let theme_color = parse_color(o.theme_color.as_deref().unwrap_or("#2196F3"), "theme-color")?;
    let background_color = parse_color(
        o.background_color.as_deref().unwrap_or("#FFFFFF"),
        "background-color",
    )?;
    let progress_color = match &o.progress_color {
        Some(c) => parse_color(c, "progress-color")?,
        None => theme_color,
    };
    let icon_background = o
        .icon_background
        .as_deref()
        .map(|c| parse_color(c, "icon-background"))
        .transpose()?;

    let mut allowed_hosts = vec![registrable_domain(&host)];
    for h in o.allowed_hosts {
        let h = h.trim().trim_start_matches("*.").to_ascii_lowercase();
        if h.is_empty() {
            continue;
        }
        if h.contains('/') || h.contains(':') {
            bail!("--allow-host expects a bare domain like example.org, got `{h}`");
        }
        if !allowed_hosts.contains(&h) {
            allowed_hosts.push(h);
        }
    }

    let min_sdk = o.min_sdk.unwrap_or(21);
    if min_sdk < 21 {
        bail!("--min-sdk must be at least 21 (Android 5.0)");
    }
    if let Some(t) = o.target_sdk
        && t < min_sdk
    {
        bail!("--target-sdk ({t}) must be >= --min-sdk ({min_sdk})");
    }

    let mut permissions = o.permissions;
    permissions.sort_by_key(|p| *p as u8);
    permissions.dedup();

    let platform = o.platform.unwrap_or(Platform::Android);
    let bundle_id = match o.bundle_id {
        Some(b) => b,
        None => package.replace('_', "-"),
    };
    validate_bundle_id(&bundle_id)?;
    let ios_min_version = o.ios_min_version.unwrap_or_else(|| "15.0".into());
    let ios_major: u32 = ios_min_version
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    if !is_numeric_version(&ios_min_version) || ios_major < 15 {
        bail!(
            "--ios-min-version must be a version like 15.0 and at least 15.0, got `{ios_min_version}`"
        );
    }
    if platform == Platform::Ios && !is_numeric_version(&version_name) {
        bail!(
            "iOS needs --version-name made of 1 to 3 numbers separated by dots (e.g. 1.2.0), got `{version_name}`"
        );
    }
    if let Some(p) = &o.provisioning_profile
        && !p.is_file()
    {
        bail!("provisioning profile not found: {}", p.display());
    }

    Ok(AppConfig {
        platform,
        allow_http: o.allow_http.unwrap_or(url.scheme() == "http"),
        url,
        name,
        package,
        version_name,
        version_code,
        icon: o.icon,
        icon_background,
        adaptive_icon: o.adaptive_icon.unwrap_or(true),
        theme_color,
        background_color,
        progress_color,
        orientation: o.orientation.unwrap_or(Orientation::Auto),
        fullscreen: o.fullscreen.unwrap_or(false),
        keep_screen_on: o.keep_screen_on.unwrap_or(false),
        pull_to_refresh: o.pull_to_refresh.unwrap_or(false),
        deep_links: o.deep_links.unwrap_or(false),
        javascript: o.javascript.unwrap_or(true),
        zoom: o.zoom.unwrap_or(false),
        downloads: o.downloads.unwrap_or(true),
        open_external_links: o.open_external_links.unwrap_or(true),
        allowed_hosts,
        user_agent: o.user_agent.unwrap_or_default(),
        user_agent_suffix: o.user_agent_suffix.unwrap_or_default(),
        debug_webview: o.debug_webview.unwrap_or(false),
        permissions,
        offline_title: o.offline_title.unwrap_or_else(|| "You're offline".into()),
        offline_message: o
            .offline_message
            .unwrap_or_else(|| "Check your internet connection and try again.".into()),
        retry_text: o.retry_text.unwrap_or_else(|| "Retry".into()),
        min_sdk,
        target_sdk: o.target_sdk,
        compile_sdk: o.compile_sdk,
        output: o.output,
        sdk: o.sdk,
        jdk: o.jdk,
        keystore: o.keystore,
        key_alias: o.key_alias,
        store_password: o.store_password,
        key_password: o.key_password,
        bundle_id,
        ios_min_version,
        simulator: o.simulator.unwrap_or(false),
        provisioning_profile: o.provisioning_profile,
        signing_identity: o.signing_identity,
    })
}

/// 1 to 3 dot-separated integers, as required by `CFBundleShortVersionString`.
fn is_numeric_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Bundle IDs allow letters, digits, `-` and `.` (no underscores).
pub fn validate_bundle_id(b: &str) -> Result<()> {
    let segments: Vec<&str> = b.split('.').collect();
    let valid = segments.len() >= 2
        && segments
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if !valid {
        bail!("invalid bundle ID `{b}` (use letters, digits, `-` and dots, e.g. com.example.app)");
    }
    Ok(())
}

pub fn parse_url(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&with_scheme).with_context(|| format!("invalid URL `{raw}`"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        bail!("URL must use http or https, got `{}`", url.scheme());
    }
    if url.host_str().is_none_or(str::is_empty) {
        bail!("URL `{raw}` has no host");
    }
    Ok(url)
}

/// Parses #RGB, #RRGGBB or #AARRGGBB into an ARGB integer.
pub fn parse_color(raw: &str, what: &str) -> Result<u32> {
    let hex = raw.trim().trim_start_matches('#');
    let expanded = match hex.len() {
        3 => format!("FF{}", hex.chars().flat_map(|c| [c, c]).collect::<String>()),
        6 => format!("FF{hex}"),
        8 => hex.to_string(),
        _ => bail!("invalid {what} `{raw}` (use #RGB, #RRGGBB or #AARRGGBB)"),
    };
    u32::from_str_radix(&expanded, 16).with_context(|| format!("invalid {what} `{raw}`"))
}

/// Best-effort "site" domain: `en.m.wikipedia.org` -> `wikipedia.org`,
/// `shop.example.co.uk` -> `example.co.uk`. IPs and single labels are kept as-is.
fn registrable_domain(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return host.to_string();
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() <= 2 {
        return host.to_string();
    }
    let n = labels.len();
    const SECOND_LEVEL: &[&str] = &[
        "co", "com", "net", "org", "gov", "edu", "ac", "or", "ne", "go", "gob", "mil", "nic",
    ];
    let keep = if labels[n - 1].len() == 2 && SECOND_LEVEL.contains(&labels[n - 2]) {
        3
    } else {
        2
    };
    labels[n.saturating_sub(keep)..].join(".")
}

fn default_name(host: &str) -> String {
    let label = host.split('.').next().unwrap_or("App");
    let mut chars = label.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => "App".into(),
    }
}

fn default_package(host: &str) -> String {
    let mut parts: Vec<String> = host
        .split('.')
        .rev()
        .filter(|s| !s.is_empty())
        .map(sanitize_package_segment)
        .collect();
    if parts.is_empty() {
        parts.push("app".into());
    }
    if parts.len() < 2 {
        parts.insert(0, "com".into());
    }
    parts.join(".")
}

fn sanitize_package_segment(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert(0, 'a');
    }
    if JAVA_KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

pub fn validate_package(p: &str) -> Result<()> {
    let segments: Vec<&str> = p.split('.').collect();
    if segments.len() < 2 {
        bail!("package `{p}` needs at least two segments, e.g. com.example.app");
    }
    for s in segments {
        let valid = s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            bail!(
                "invalid package segment `{s}` in `{p}` (letters, digits and _ only; must start with a letter)"
            );
        }
        if JAVA_KEYWORDS.contains(&s) {
            bail!("package segment `{s}` in `{p}` is a reserved Java keyword");
        }
    }
    Ok(())
}

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
    "true",
    "false",
    "null",
    "var",
    "record",
    "yield",
];

pub fn init_template(url: &str) -> String {
    format!(
        r##"# url-to-apk configuration. Build with: url-to-apk build
# Every key is optional except `url`; CLI flags override values in this file.

url = "{url}"
# platform = "android"    # android | ios (iOS builds need macOS + Xcode)
# name = "My App"
# package = "com.example.app"
version_name = "1.0.0"
version_code = 1          # increase for every release

# --- Appearance ---
# icon = "icon.png"       # PNG/JPEG/WebP, 512x512 recommended
# icon_background = "#FFFFFF"
# adaptive_icon = true
theme_color = "#2196F3"
background_color = "#FFFFFF"
# progress_color = "#2196F3"
orientation = "auto"      # auto | portrait | landscape | sensor
fullscreen = false
keep_screen_on = false

# --- Behavior ---
pull_to_refresh = false
deep_links = false
javascript = true
zoom = false
downloads = true
open_external_links = true
# allowed_hosts = ["accounts.google.com", "cdn.example.com"]
# user_agent_suffix = "MyApp/1.0"
# allow_http = false
debug_webview = false
permissions = []          # any of: "location", "camera", "microphone"

# --- Offline screen text ---
# offline_title = "You're offline"
# offline_message = "Check your internet connection and try again."
# retry_text = "Retry"

# --- Build ---
min_sdk = 21
# target_sdk = 35
# compile_sdk = 35
# output = "dist/app.apk"
# sdk = "/path/to/Android/Sdk"
# jdk = "/path/to/jdk"

# --- Signing (default: auto-generated ~/.url-to-apk/default.keystore) ---
# keystore = "release.keystore"
# key_alias = "release"
# Passwords: set URL2APK_STORE_PASSWORD / URL2APK_KEY_PASSWORD env vars

# --- iOS (used with platform = "ios" or --platform ios) ---
# bundle_id = "com.example.app"   # default: package with `_` -> `-`
# ios_min_version = "15.0"
# simulator = false               # true: build a .app for the iOS Simulator
# provisioning_profile = "MyApp.mobileprovision"   # needed to install on devices
# signing_identity = "Apple Development"
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff", "c").unwrap(), 0xFFFFFFFF);
        assert_eq!(parse_color("2196F3", "c").unwrap(), 0xFF2196F3);
        assert_eq!(parse_color("#80000000", "c").unwrap(), 0x80000000);
        assert!(parse_color("#12345", "c").is_err());
    }

    #[test]
    fn defaults_from_host() {
        assert_eq!(default_package("my-site.co.uk"), "uk.co.my_site");
        assert_eq!(default_package("localhost"), "com.localhost");
        assert_eq!(default_package("123.example.com"), "com.example.a123");
        assert_eq!(default_name("example.com"), "Example");
        validate_package(&default_package("new.example.com")).unwrap();
    }

    #[test]
    fn registrable() {
        assert_eq!(registrable_domain("en.m.wikipedia.org"), "wikipedia.org");
        assert_eq!(registrable_domain("shop.example.co.uk"), "example.co.uk");
        assert_eq!(registrable_domain("example.com"), "example.com");
        assert_eq!(registrable_domain("192.168.1.10"), "192.168.1.10");
        assert_eq!(registrable_domain("localhost"), "localhost");
    }

    #[test]
    fn packages() {
        assert!(validate_package("com.example.app").is_ok());
        assert!(validate_package("app").is_err());
        assert!(validate_package("com.1x.app").is_err());
        assert!(validate_package("com.new.app").is_err());
    }

    #[test]
    fn ios_values() {
        assert!(is_numeric_version("1.2.0"));
        assert!(is_numeric_version("15"));
        assert!(!is_numeric_version("1.0.0-beta"));
        assert!(!is_numeric_version("1.2.3.4"));
        assert!(validate_bundle_id("com.my-site.app").is_ok());
        assert!(validate_bundle_id("com.my_site.app").is_err());
        assert!(validate_bundle_id("app").is_err());
    }

    #[test]
    fn urls() {
        assert_eq!(
            parse_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert!(parse_url("ftp://example.com").is_err());
    }
}
