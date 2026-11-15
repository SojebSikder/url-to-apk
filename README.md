# url2apk

Turn any website into an installable Android app or iOS app (a WebView wrapper) with
one command.

```sh
url2apk build https://example.com                                       # Android APK
url2apk build https://example.com --platform ios --simulator --install  # iOS (macOS only)
```

No Gradle, no Android Studio or Xcode project. The tool generates a tiny dependency-free
app and builds it directly with the platform tools:

- **Android:** `aapt2` -> `javac` -> `d8` -> `zipalign` -> `apksigner`. A typical APK is
  ~40–100 KB and builds in a few seconds, on Windows, macOS or Linux.
- **iOS:** `swiftc` -> `actool` -> `codesign`, producing an `.ipa` (devices) or `.app`
  (Simulator). Needs macOS with Xcode. See [iOS](#ios).

## Requirements

### For Android

| Tool                    | Notes                                                                                  |
| ----------------------- | -------------------------------------------------------------------------------------- |
| JDK 17+                 | Found via `--jdk`, `JAVA_HOME`, the `java` on `PATH`, or Android Studio's bundled JBR  |
| Android SDK             | Found via `--sdk`, `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or the default install location |
| `build-tools`           | Any recent version: `sdkmanager "build-tools;35.0.0"`                                  |
| `platforms;android-33`+ | `sdkmanager "platforms;android-35"`                                                    |
| `platform-tools`        | Optional, only for `--install`                                                         |

### For iOS

| Tool  | Notes                                                                                         |
| ----- | --------------------------------------------------------------------------------------------- |
| macOS | Apple's SDK and signing tools only run on macOS (a Mac or a CI runner like `macos-latest`)    |
| Xcode | The full app, not just the Command Line Tools: `sudo xcode-select -s /Applications/Xcode.app` |

Check everything with:

```sh
url2apk doctor                  # Android
url2apk doctor --platform ios   # iOS
```

## Install

Download a prebuilt binary for Windows, macOS or Linux from the
[Releases](https://github.com/SojebSikder/url2apk/releases) page and put it on your
`PATH`. Or build it with Rust:

```sh
cargo install --git https://github.com/SojebSikder/url2apk
# or, from a clone of this repo:
cargo install --path .
```

## Usage

```sh
# Simplest: name, package and icon are derived from the domain
url2apk build https://example.com

# Customized
url2apk build https://example.com \
  --name "Example" --package com.example.app \
  --icon logo.png --theme-color "#0F9D58" \
  --version-name 1.2.0 --version-code 12 \
  --pull-to-refresh --deep-links \
  --permission camera --permission location \
  --install            # adb install + launch on a connected device

# The same app for iOS (on macOS)
url2apk build https://example.com --platform ios \
  --name "Example" --bundle-id com.example.app \
  --icon logo.png --theme-color "#0F9D58" \
  --version-name 1.2.0 --version-code 12 \
  --simulator --install   # install + launch on the booted Simulator

# Config-file workflow (good for repeatable builds / CI)
url2apk init https://example.com   # writes a documented url2apk.toml
url2apk build                      # picks up ./url2apk.toml automatically
url2apk build --platform ios       # same config, iOS build
url2apk init --ci github           # adds a GitHub Actions workflow (see below)
```

CLI flags override values in the config file. Boolean flags accept `--flag` or
`--flag=false`. Run `url2apk build --help` for every option.

## Build on GitHub Actions

Build both apps in the cloud, including iOS without owning a Mac:

```sh
url2apk init https://example.com --ci github
```

This writes `url2apk.toml` and `.github/workflows/url2apk.yml`. If `url2apk.toml` already
exists, it's kept. You can also copy [`examples/github-actions.yml`](examples/github-actions.yml)
by hand. Commit both files, then run the workflow from the **Actions** tab or push a tag:

```sh
git tag v1.0.0 && git push origin v1.0.0
```

Signing comes from repository secrets (**Settings > Secrets and variables > Actions**). All
are optional:

| Secret                   | Value                                                                                      |
| ------------------------ | ------------------------------------------------------------------------------------------ |
| `ANDROID_KEYSTORE_B64`   | Your keystore, base64-encoded (create it once with `url2apk keygen`)                       |
| `ANDROID_STORE_PASSWORD` | The keystore password                                                                      |
| `ANDROID_KEY_ALIAS`      | The key alias (default `release`)                                                          |
| `IOS_CERT_P12_B64`       | Your Apple Development/Distribution certificate exported from Keychain as `.p12`, base64   |
| `IOS_CERT_PASSWORD`      | The `.p12` password                                                                        |
| `IOS_PROFILE_B64`        | The `.mobileprovision` matching `bundle_id`, base64                                        |
| `URL2APK_TOKEN`          | Only while this repo is private: a fine-grained token with read-only Contents access to it |

- Without the Android keystore, each run signs with a new throwaway key, so a new APK can't
  be installed over an older one. The workflow shows a warning when this happens.
- Without the iOS secrets you get an ad-hoc signed `.ipa`. Re-sign it with Sideloadly,
  AltStore, etc. to install it on a device.

## What the generated app does

This section describes the Android app. The iOS app has the same features with
iOS-native behavior. See [iOS](#ios) for the differences.

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

## Signing (Android)

By default, APKs are signed with a key generated once at
`~/.url2apk/default.keystore`. Android only accepts an update signed with the
**same key**, so keep that file, or use your own release key:

```sh
URL2APK_STORE_PASSWORD='s3cret!' url2apk keygen -o release.keystore -a release
URL2APK_STORE_PASSWORD='s3cret!' url2apk build https://example.com \
  --keystore release.keystore --key-alias release
```

Back up your keystore and password. If you lose them, you can't update the app.

## iOS

`--platform ios` (or `platform = "ios"` in `url2apk.toml`) builds a native WKWebView app
from the same options. The tool generates a small Swift app and builds it with the Xcode
command-line tools (`swiftc` -> `actool` -> `codesign`), so there's no Xcode project.

You need macOS with Xcode (see [Requirements](#for-ios)). On Windows or Linux, use a Mac or a
macOS CI runner such as GitHub Actions `runs-on: macos-latest`.

```sh
# Simulator: builds Example-1.0.0.app, installs it on the booted Simulator and launches it
open -a Simulator
url2apk build https://example.com --platform ios --simulator --install

# Device: a signed .ipa for your development or distribution profile
url2apk build https://example.com --platform ios \
  --bundle-id com.example.app --provisioning-profile MyApp.mobileprovision

# Without a profile you get an ad-hoc signed .ipa for re-signing tools (Sideloadly, AltStore, ...)
url2apk build https://example.com --platform ios
```

| Option                   | Notes                                                                                    |
| ------------------------ | ---------------------------------------------------------------------------------------- |
| `--bundle-id`            | Defaults to `--package` with `_` replaced by `-`; must match the provisioning profile    |
| `--ios-min-version`      | Default `15.0`                                                                           |
| `--simulator`            | Build a `.app` for the Simulator instead of an `.ipa`                                    |
| `--provisioning-profile` | Entitlements and team come from the profile; wildcard profiles work                      |
| `--signing-identity`     | Defaults to "Apple Development" for development profiles, otherwise "Apple Distribution" |

`--version-name` becomes `CFBundleShortVersionString`, so for iOS it must be 1 to 3 numbers
separated by dots (e.g. `1.2.0`). `--version-code` becomes `CFBundleVersion`.

iOS differences:

- `--deep-links` is ignored. Universal Links need an Associated Domains entitlement and an
  `apple-app-site-association` file on your site.
- Android-only options (`--min-sdk`, `--adaptive-icon`, `--keystore`, ...) are ignored.
- Camera and microphone usage descriptions are always included, because the file picker's
  "Take Photo/Video" option needs them.
- `--install` only works with `--simulator`. For a device, install the `.ipa` with Xcode
  (Window > Devices and Simulators) or `xcrun devicectl device install app --device <UDID> <ipa>`.
- App Store uploads need a distribution profile. Apple may reject apps that only wrap a
  website (App Review Guideline 4.2).

## Notes and limitations

- Android output is an APK, which is right for sideloading and most stores. Google Play
  requires an AAB for new apps; convert with
  [bundletool](https://developer.android.com/tools/bundletool) or a Gradle project.
- On Android 12+, `https` deep links open the app only after the user enables them in
  the app's "Open by default" settings. Automatic verification requires hosting
  `/.well-known/assetlinks.json` on your domain.
- Pull-to-refresh triggers when the page is scrolled to the top. Sites that scroll an
  inner container instead of the document can trigger it while scrolled.
- On Android, `blob:` and `data:` URL downloads aren't supported.
- `--keep-build DIR` keeps the generated project for inspection; `-v` prints every tool
  invocation.
