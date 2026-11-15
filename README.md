# url-to-apk

Turn any website into an installable Android app (a WebView wrapper) with one command.

```sh
url-to-apk build https://example.com
```

No Gradle, no Android Studio project. The tool generates a tiny dependency-free
Android app and builds it directly with the Android SDK tools
(`aapt2` → `javac` → `d8` → `zipalign` → `apksigner`). A typical APK is ~40–100 KB
and builds in a few seconds.

## Requirements

| Tool                    | Notes                                                                                  |
| ----------------------- | -------------------------------------------------------------------------------------- |
| JDK 17+                 | Found via `--jdk`, `JAVA_HOME`, the `java` on `PATH`, or Android Studio's bundled JBR  |
| Android SDK             | Found via `--sdk`, `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or the default install location |
| `build-tools`           | Any recent version: `sdkmanager "build-tools;35.0.0"`                                  |
| `platforms;android-33`+ | `sdkmanager "platforms;android-35"`                                                    |
| `platform-tools`        | Optional, only for `--install`                                                         |

Check everything with:

```sh
url-to-apk doctor
```

## Install

```sh
cargo install --path .
```

## Usage

```sh
# Simplest: name, package and icon are derived from the domain
url-to-apk build https://example.com

# Customized
url-to-apk build https://example.com \
  --name "Example" --package com.example.app \
  --icon logo.png --theme-color "#0F9D58" \
  --version-name 1.2.0 --version-code 12 \
  --pull-to-refresh --deep-links \
  --permission camera --permission location \
  --install            # adb install + launch on a connected device

# Config-file workflow (good for repeatable builds / CI)
url-to-apk init https://example.com   # writes a documented url2apk.toml
url-to-apk build                      # picks up ./url2apk.toml automatically
```

CLI flags override values in the config file. Boolean flags accept `--flag` or
`--flag=false`. Run `url-to-apk build --help` for every option.

## What the generated app does

- **Navigation:** back button walks web history; links to other domains open in the
  browser (configurable with `--allow-host` / `--open-external-links=false`); server
  redirects (login/SSO) always stay in-app
- **App links:** `tel:`, `mailto:`, `sms:`, `geo:`, `market:`, WhatsApp, and
  `intent://` (with `browser_fallback_url`) open the right app
- **Offline:** cached pages load when offline; otherwise a themed "You're offline"
  screen with a Retry button (text is customizable)
- **File uploads:** `<input type=file>` with multiple files, `accept=` filtering, and
  camera capture (Android 10+, with `--permission camera`)
- **Downloads:** through the system DownloadManager, with session cookies and user agent
- **Permissions:** geolocation, camera, and microphone (WebRTC/getUserMedia), asked at
  runtime only when the site requests them
- **Video:** HTML5 fullscreen video
- **Appearance:** themed status/navigation bars (edge-to-edge with correct insets,
  keyboard-aware), Android 12+ splash color, light/dark bar icons, immersive
  `--fullscreen`, orientation lock, `--keep-screen-on`
- **Pull to refresh** (`--pull-to-refresh`)
- **Deep links** (`--deep-links`): links to your site can open in the app
- **Icons:** legacy and adaptive icons from any PNG/JPEG/WebP (or a generated default)
- **Robustness:** state preserved on rotation; recovers from WebView renderer crashes;
  cookies persisted
- **Debugging:** `--debug-webview` enables `chrome://inspect`

## Signing

By default, APKs are signed with a key generated once at
`~/.url-to-apk/default.keystore`. Android only accepts an update signed with the
**same key**, so keep that file, or use your own release key:

```sh
URL2APK_STORE_PASSWORD='s3cret!' url-to-apk keygen -o release.keystore -a release
URL2APK_STORE_PASSWORD='s3cret!' url-to-apk build https://example.com \
  --keystore release.keystore --key-alias release
```

Back up your keystore and password. If you lose them, you can't update the app.

## Notes and limitations

- Output is an APK, which is right for sideloading and most stores. Google Play
  requires an AAB for new apps; convert with
  [bundletool](https://developer.android.com/tools/bundletool) or a Gradle project.
- On Android 12+, `https` deep links open the app only after the user enables them in
  the app's "Open by default" settings. Automatic verification requires hosting
  `/.well-known/assetlinks.json` on your domain.
- Pull-to-refresh triggers when the page is scrolled to the top. Sites that scroll an
  inner container instead of the document can trigger it while scrolled.
- `blob:` and `data:` URL downloads aren't supported.
- `--keep-build DIR` keeps the generated project for inspection; `-v` prints every tool
  invocation.
