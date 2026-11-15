//! Writes the Android project (manifest, resources, Java sources) to disk.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{AppConfig, Permission};
use crate::icon;

const MAIN_ACTIVITY: &str = include_str!("templates/MainActivity.java");

pub struct Project {
    pub manifest: PathBuf,
    pub res: PathBuf,
    pub java_sources: Vec<PathBuf>,
}

pub fn generate(cfg: &AppConfig, dir: &Path, compile_sdk: u32) -> Result<Project> {
    let res = dir.join("res");
    let values = res.join("values");
    std::fs::create_dir_all(&values)?;

    let icons = icon::write_icons(
        &res,
        cfg.icon.as_deref(),
        cfg.theme_color,
        cfg.icon_background,
    )?;
    if cfg.adaptive_icon {
        let anydpi = res.join("mipmap-anydpi-v26");
        std::fs::create_dir_all(&anydpi)?;
        std::fs::write(anydpi.join("ic_launcher.xml"), ADAPTIVE_ICON_XML)?;
    }

    std::fs::write(
        values.join("strings.xml"),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    <string name=\"app_name\">{}</string>\n</resources>\n",
            android_string(&cfg.name)
        ),
    )?;
    std::fs::write(
        values.join("colors.xml"),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    \
             <color name=\"theme\">{}</color>\n    \
             <color name=\"background\">{}</color>\n    \
             <color name=\"ic_launcher_background\">{}</color>\n</resources>\n",
            hex(cfg.theme_color),
            hex(cfg.background_color),
            hex(icons.adaptive_background)
        ),
    )?;
    std::fs::write(values.join("themes.xml"), themes_xml(cfg))?;
    if compile_sdk >= 31 {
        let v31 = res.join("values-v31");
        std::fs::create_dir_all(&v31)?;
        std::fs::write(
            v31.join("themes.xml"),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n    \
             <style name=\"AppTheme\" parent=\"BaseTheme\">\n        \
             <item name=\"android:windowSplashScreenBackground\">@color/background</item>\n    \
             </style>\n</resources>\n",
        )?;
    }

    let manifest = dir.join("AndroidManifest.xml");
    std::fs::write(&manifest, manifest_xml(cfg))?;

    let src = dir.join("src").join(cfg.package.replace('.', "/"));
    std::fs::create_dir_all(&src)?;
    let activity = src.join("MainActivity.java");
    std::fs::write(
        &activity,
        MAIN_ACTIVITY.replace("__PACKAGE__", &cfg.package),
    )?;
    let config = src.join("Config.java");
    std::fs::write(&config, config_java(cfg))?;

    Ok(Project {
        manifest,
        res,
        java_sources: vec![activity, config],
    })
}

const ADAPTIVE_ICON_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@color/ic_launcher_background"/>
    <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
</adaptive-icon>
"#;

fn is_light(argb: u32) -> bool {
    let (r, g, b) = ((argb >> 16) & 0xFF, (argb >> 8) & 0xFF, argb & 0xFF);
    (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) / 255.0 > 0.6
}

fn themes_xml(cfg: &AppConfig) -> String {
    let parent = if is_light(cfg.background_color) {
        "android:Theme.Material.Light.NoActionBar"
    } else {
        "android:Theme.Material.NoActionBar"
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<resources>
    <style name="BaseTheme" parent="{parent}">
        <item name="android:colorPrimary">@color/theme</item>
        <item name="android:colorPrimaryDark">@color/theme</item>
        <item name="android:colorAccent">@color/theme</item>
        <item name="android:windowBackground">@color/background</item>
        <item name="android:statusBarColor">@color/theme</item>
        <item name="android:navigationBarColor">@color/theme</item>
        <item name="android:windowFullscreen">{fullscreen}</item>
    </style>
    <style name="AppTheme" parent="BaseTheme"/>
</resources>
"#,
        fullscreen = cfg.fullscreen
    )
}

fn manifest_xml(cfg: &AppConfig) -> String {
    let mut perms = String::new();
    let mut add = |name: &str, extra: &str| {
        let _ = writeln!(
            perms,
            "    <uses-permission android:name=\"android.permission.{name}\"{extra}/>"
        );
    };
    add("INTERNET", "");
    add("ACCESS_NETWORK_STATE", "");
    if cfg.downloads {
        add("WRITE_EXTERNAL_STORAGE", " android:maxSdkVersion=\"28\"");
    }
    if cfg.has_permission(Permission::Location) {
        add("ACCESS_FINE_LOCATION", "");
        add("ACCESS_COARSE_LOCATION", "");
    }
    if cfg.has_permission(Permission::Camera) {
        add("CAMERA", "");
    }
    if cfg.has_permission(Permission::Microphone) {
        add("RECORD_AUDIO", "");
        add("MODIFY_AUDIO_SETTINGS", "");
    }

    let mut features = String::new();
    if cfg.has_permission(Permission::Camera) {
        features.push_str("    <uses-feature android:name=\"android.hardware.camera\" android:required=\"false\"/>\n");
    }
    if cfg.has_permission(Permission::Location) {
        features.push_str("    <uses-feature android:name=\"android.hardware.location.gps\" android:required=\"false\"/>\n");
    }
    if cfg.has_permission(Permission::Microphone) {
        features.push_str("    <uses-feature android:name=\"android.hardware.microphone\" android:required=\"false\"/>\n");
    }

    let deep_link = if cfg.deep_links {
        let host = cfg.base_host();
        let mut hosts = vec![host.clone()];
        if host.parse::<std::net::IpAddr>().is_err() && host.contains('.') {
            hosts.push(format!("www.{host}"));
        }
        let mut data = String::from(
            "                <data android:scheme=\"https\"/>\n                <data android:scheme=\"http\"/>\n",
        );
        for h in hosts {
            let _ = writeln!(
                data,
                "                <data android:host=\"{}\"/>",
                xml_escape(&h)
            );
        }
        format!(
            "            <intent-filter>\n                \
             <action android:name=\"android.intent.action.VIEW\"/>\n                \
             <category android:name=\"android.intent.category.DEFAULT\"/>\n                \
             <category android:name=\"android.intent.category.BROWSABLE\"/>\n{data}            </intent-filter>\n"
        )
    } else {
        String::new()
    };

    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{package}">

{perms}{features}
    <application
        android:label="@string/app_name"
        android:icon="@mipmap/ic_launcher"
        android:theme="@style/AppTheme"
        android:allowBackup="true"
        android:hardwareAccelerated="true"
        android:supportsRtl="true"
        android:usesCleartextTraffic="{cleartext}">
        <activity
            android:name=".MainActivity"
            android:exported="true"
            android:launchMode="singleTask"
            android:screenOrientation="{orientation}"
            android:windowSoftInputMode="adjustResize"
            android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|navigation|uiMode">
            <intent-filter>
                <action android:name="android.intent.action.MAIN"/>
                <category android:name="android.intent.category.LAUNCHER"/>
            </intent-filter>
{deep_link}        </activity>
    </application>
</manifest>
"#,
        package = cfg.package,
        cleartext = cfg.allow_http,
        orientation = cfg.orientation.manifest_value(),
    )
}

fn config_java(cfg: &AppConfig) -> String {
    let hosts = cfg
        .allowed_hosts
        .iter()
        .map(|h| java_string(h))
        .collect::<Vec<_>>()
        .join(", ");
    let b = |v: bool| if v { "true" } else { "false" };
    format!(
        r#"package {package};

/** Generated by url-to-apk. Do not edit. */
final class Config {{
    private Config() {{}}

    static final String START_URL = {url};
    static final String[] ALLOWED_HOSTS = {{ {hosts} }};
    static final boolean OPEN_EXTERNAL_LINKS = {external};
    static final boolean JAVASCRIPT = {js};
    static final boolean ZOOM = {zoom};
    static final boolean DOWNLOADS = {downloads};
    static final boolean FULLSCREEN = {fullscreen};
    static final boolean KEEP_SCREEN_ON = {keep_on};
    static final boolean PULL_TO_REFRESH = {ptr};
    static final boolean DEBUG_WEBVIEW = {debug};
    static final boolean GEOLOCATION = {geo};
    static final boolean CAMERA = {camera};
    static final boolean MICROPHONE = {mic};
    static final String USER_AGENT = {ua};
    static final String USER_AGENT_SUFFIX = {ua_suffix};
    static final int THEME_COLOR = 0x{theme:08X};
    static final int BACKGROUND_COLOR = 0x{bg:08X};
    static final int PROGRESS_COLOR = 0x{progress:08X};
    static final String OFFLINE_TITLE = {offline_title};
    static final String OFFLINE_MESSAGE = {offline_message};
    static final String RETRY_TEXT = {retry};
}}
"#,
        package = cfg.package,
        url = java_string(cfg.url.as_str()),
        external = b(cfg.open_external_links),
        js = b(cfg.javascript),
        zoom = b(cfg.zoom),
        downloads = b(cfg.downloads),
        fullscreen = b(cfg.fullscreen),
        keep_on = b(cfg.keep_screen_on),
        ptr = b(cfg.pull_to_refresh),
        debug = b(cfg.debug_webview),
        geo = b(cfg.has_permission(Permission::Location)),
        camera = b(cfg.has_permission(Permission::Camera)),
        mic = b(cfg.has_permission(Permission::Microphone)),
        ua = java_string(&cfg.user_agent),
        ua_suffix = java_string(&cfg.user_agent_suffix),
        theme = cfg.theme_color,
        bg = cfg.background_color,
        progress = cfg.progress_color,
        offline_title = java_string(&cfg.offline_title),
        offline_message = java_string(&cfg.offline_message),
        retry = java_string(&cfg.retry_text),
    )
}

fn hex(argb: u32) -> String {
    format!("#{argb:08X}")
}

/// Java string literal. Non-ASCII is kept as-is (sources are compiled as UTF-8);
/// control characters use octal escapes because `\uXXXX` is processed before lexing.
fn java_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\{:03o}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Escapes text for an Android `<string>` resource (aapt escaping + XML).
fn android_string(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '@' | '?' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    xml_escape(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(java_string("a\"b\\c\nd\u{1}"), r#""a\"b\\c\nd\001""#);
        assert_eq!(
            android_string("Tom's <App> & \"co\""),
            r#"Tom\'s &lt;App&gt; &amp; \&quot;co\&quot;"#
        );
        assert_eq!(android_string("@home"), "\\@home");
    }
}
