# Onboarding — Clean Machine Setup (Phase 11)

## Status of this document

The Linux-side build commands below were run for real in this repo while
writing this doc — `cargo build`, `cargo test`, `./gradlew assembleDebug`,
and `./gradlew assembleRelease` all genuinely succeed as described. What
was **not** done: running this flow end-to-end on an actual clean Linux
install and a factory-reset Samsung phone, which is what `docs/SYSTEM_DESIGN.md`'s
Phase 11 DoD asks for. Treat this as a verified-buildable, not yet
verified-on-clean-hardware, walkthrough — that last step is real physical
hardware testing this environment can't do.

## 1. Linux host

### Prerequisites
- Rust (stable) via [rustup](https://rustup.rs).
- For the GUI build (mirror window, preferences, notification toasts):
  `libgtk-4-dev`, `libadwaita-1-dev`, and GStreamer development packages
  (`libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`,
  `gstreamer1.0-plugins-good`, `gstreamer1.0-plugins-bad`,
  `gstreamer1.0-vaapi` for hardware decode).
- For the file-mount feature: `/dev/fuse` access and `fusermount3`.

### Build
```sh
git clone <repo-url> hyperlink && cd hyperlink

# Headless daemon (no GUI dependencies required):
cargo build --release -p hyperlink-linux

# Full GUI build (mirror window + preferences + notifications):
cargo build --release -p hyperlink-linux --features video
```

### Run and pair
```sh
# First run: pair a phone.
./target/release/hyperlink-linux --pair
```
The terminal prints a 6-digit PIN and a certificate fingerprint, then asks
`Do you trust this device? (y/n)`. Compare the PIN against what the Android
companion app shows on its own pairing screen *before* answering — that
comparison is the only thing standing between TOFU pairing and a
man-in-the-middle (see `docs/SECURITY_REVIEW.md`). The pairing window closes
automatically after one resolved attempt or 5 minutes, whichever comes
first — run `--pair` again to pair a second device.

```sh
# Subsequent runs — normal mode, only previously-paired devices are accepted:
./target/release/hyperlink-linux
```

### Optional: install as a systemd user service
```sh
mkdir -p ~/.local/bin ~/.config/systemd/user
cp target/release/hyperlink-linux ~/.local/bin/
cp linux/packaging/systemd/hyperlink.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now hyperlink.service
journalctl --user -u hyperlink.service -f   # watch logs
```
Note: the systemd unit runs the **headless** binary — use the GUI build
(`--features video`) as a normal desktop app, launched separately, if you
want the mirror window and preferences UI. Running both against the same
bind port at once will conflict.

### Optional: Flatpak
See `linux/packaging/flatpak/com.hyperlink.Host.yml` — it documents the
offline-vendoring step required before `flatpak-builder` can build it; that
manifest has not yet been build-verified (no `flatpak-builder` in the
environment this was authored in).

## 2. Android companion

### Build
```sh
cd android
./gradlew assembleDebug     # -> app/build/outputs/apk/debug/app-debug.apk
```
This was run in this repo and succeeds. Two prerequisite fixes were needed
to get here and are now part of the tree: the `androidx.core:core-ktx`
dependency + `FileProvider` manifest/resource entries that
`ClipboardService` needed, and two small MainActivity helper-method gaps
(`createSecondaryButton`, a `logMessage`→`log` typo) — without those the
module didn't compile at all before this pass.

A release build (`./gradlew assembleRelease`) also succeeds, R8-minified,
but **falls back to debug signing** without a real keystore — see
`android/keystore.properties.example` for how to produce one, and don't
distribute a debug-signed release build to real users.

### Install
```sh
adb install -r app/build/outputs/apk/debug/app-debug.apk
```
(A signed release APK, once a real keystore exists, installs the same way —
this repo has no Play Store / internal-test-track publishing set up yet.)

### Grant permissions (all one-time, manual — this is the "zero adb steps for
an end user" gap called out below)
On first launch, HyperLink needs several permissions granted through
Android's own Settings UI, not anything this app can request programmatically
end-to-end:
1. **Screen capture** — granted per-session via the system's
   `MediaProjection` consent dialog when you tap "Start Mirroring".
2. **Accessibility Service** (for input injection) — Settings → Accessibility
   → HyperLink → enable. Required before mouse/keyboard/touch from the PC
   does anything.
3. **Notification Access** — tap "Notification & DND Permissions" in the app,
   or Settings → Apps → Special app access → Notification access → HyperLink.
4. **Do Not Disturb access** — same screen, needed for DND sync in both
   directions.
5. **Storage access** — tap "Storage & File Permissions" in the app
   (`MANAGE_EXTERNAL_STORAGE` on Android 11+) — required for the virtual file
   mount.

## 3. Verify the pairing + mirror flow
1. Start the Linux host with `--pair`.
2. Launch the Android app; it should discover the host via mDNS (same LAN)
   and show the PIN. Confirm the PIN matches, accept on both sides.
3. Grant the Accessibility Service and Notification Access permissions
   (step 2/3 above) if not already done.
4. Tap "Start Mirroring" on the phone. The Linux mirror window should open
   and show the phone's screen within a few seconds.
5. Click inside the mirror window — the corresponding tap should register on
   the phone.
6. Open Preferences (gear icon in the mirror window's header bar) to confirm
   feature toggles, bandwidth cap, crash-report location, and trusted-device
   list all load correctly.

## Known gaps against the Phase 11 DoD
- **"Zero manual adb/dev-tool steps"**: not met. The permission grants in
  step 2 above are unavoidable manual taps through Android's own Settings —
  none of them are `adb` commands, but they are manual, multi-screen steps a
  first-run wizard inside the app could streamline (deep-linking directly to
  each relevant Settings screen) but currently doesn't.
- **Signed release APK**: scaffolding is in place (`keystore.properties.example`,
  working R8 config) but no real signing key exists, so there is no
  distributable signed build yet.
- **Flatpak**: manifest written, not build-verified.
- **Clean-hardware test**: not performed — see the status note at the top.
