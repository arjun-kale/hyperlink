# Releasing HyperLink

Releases are built by GitHub Actions ([`.github/workflows/release.yml`](../.github/workflows/release.yml))
when a version tag is pushed. Each release has:

- `hyperlink-linux-<tag>-x86_64.tar.gz` — the Linux app, icons, app-menu entry
  and the installer script (`linux/packaging/install-local.sh --no-build`).
  Built on Ubuntu 24.04, so it runs on 24.04 and newer.
- `HyperLink-<tag>.apk` — the Android app, signed with the release key.

## One-time: create the Android release key

Android identifies an app by its signing key: every update must be signed
with the **same** key, forever. If it's lost, existing users can't update and
have to uninstall and reinstall. Create it once and back it up somewhere safe
(a password manager plus an offline copy).

```sh
keytool -genkeypair -v -keystore hyperlink-release.keystore -alias hyperlink \
  -keyalg RSA -keysize 4096 -validity 10000
```

Then store it as repository secrets so CI can sign releases
(Settings → Secrets and variables → Actions, or with the `gh` CLI):

```sh
gh secret set ANDROID_KEYSTORE_BASE64 < <(base64 -w0 hyperlink-release.keystore)
gh secret set ANDROID_KEYSTORE_PASSWORD
gh secret set ANDROID_KEY_ALIAS --body hyperlink
gh secret set ANDROID_KEY_PASSWORD
```

Without these secrets the workflow still runs, but the APK is debug-signed and
the release is marked as a pre-release. Note that switching from a
debug-signed build to the release key is a key change: phones with the
debug-signed app installed must uninstall it first.

For local release builds, copy `android/keystore.properties.example` to
`android/keystore.properties` and point it at the keystore (both are
git-ignored).

## Cutting a release

1. Update the version in `linux/Cargo.toml`, `protocol/Cargo.toml`,
   `bench/Cargo.toml`, `android-bridge/Cargo.toml`, and `versionName` /
   `versionCode` (increment by 1) in `android/app/build.gradle.kts`.
2. Add a section to `CHANGELOG.md` and a `<release>` entry to
   `linux/packaging/flatpak/com.hyperlink.Host.appdata.xml`.
   The Linux app's update check reads the newest `## [x.y.z]` heading from
   `CHANGELOG.md` on `main`, so merging this is what tells users an update exists.
3. If the Rust code in `android-bridge/` changed, rebuild the committed
   `android/app/src/main/jniLibs/arm64-v8a/libhyperlink_bridge.so`.
4. If `protocol/` changed in a way older apps can't understand, bump
   `PROTOCOL_VERSION` in `protocol/src/version.rs`. Mismatched phones and
   computers then refuse to connect with a clear "different versions" message
   instead of misbehaving.
5. Merge to `main`, then tag and push:

   ```sh
   git tag v0.3.0 && git push origin v0.3.0
   ```

## Google Play

Play distribution avoids the sideloading blocks described in
[ONBOARDING.md](ONBOARDING.md#installing-the-apk). It needs a Play Console
developer account, the release key above (enrolled in Play App Signing), and
these listing items:

- **Privacy policy:** [PRIVACY.md](../PRIVACY.md), published at a public URL.
- **Accessibility API declaration:** HyperLink uses it for remote control from
  the user's own computer and as a clipboard-sync trigger; both must be
  disclosed in the app's listing and in-app before the user enables it.
- **Notification listener and "All files access"** declarations, explaining
  that they power notification mirroring and file browsing from the user's
  computer.
- **Data safety form:** no data is collected or shared with third parties; all
  data stays on the user's devices (see PRIVACY.md).
- **Target API level:** the app targets Android 16 (API 36).
