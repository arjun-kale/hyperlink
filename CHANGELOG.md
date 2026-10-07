# Changelog

All notable changes to this project are documented here. Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.3.0] - 2026-10-07
### Added
- Linux desktop app: one window for pairing, linked status and the phone's screen, replacing the terminal flow. Pairing starts from a button; you pick the code your phone shows from three, instead of answering y/n.
- Redesigned Android app (Jetpack Compose): welcome, find-your-computer, pairing and linked screens, a permission checklist, and plain-language messages for every failure.
- The phone stays linked with the app closed, and reconnects on its own for 5 minutes after a drop. The "Linked" notification opens the app and has a Disconnect button.
- Phone and computer exchange protocol versions and device names on connect; mismatched versions refuse to connect with a clear message, and the computer shows the phone by its real name.
- App icons from the HyperLink logo, an app-menu entry and installer for Linux, a release workflow that publishes both apps, a privacy policy, and a setup guide.
### Changed
- Screen sharing tuned for latency: hardware decoding (VA-API), no clock waits in the display path, frames bypass the UI thread, low-latency encoder settings at 60 fps, Wi-Fi power save off while sharing, and lost frames recover with an immediate keyframe instead of waiting up to a second.
- The Android app targets Android 16 (API 36).
### Fixed
- Clipboard polling made GNOME Shell use ~90% CPU and froze the desktop; the app now uses GTK's clipboard notifications.
- A blocking clipboard helper could stall the computer's connection server and drop the session.
- The phone never accepted the computer's clipboard and file streams, which also made the computer's window freeze.
- Screen sharing crashed the phone app (`RESULT_OK` was treated as an error), and on some Samsung encoders the video never started.
- The files mount never stayed mounted.
- Pairing a second phone or computer replaced the first one's trust.
- Toast timeouts dismissed notifications on the phone.
- The update check pointed at the wrong repository.
- App data (including the pairing key) is excluded from Android backup and device transfer.
- Android CI could never fail (build errors were swallowed).

## [0.2.0] - 2026-08-26
### Added
- **Phase 1 complete**: Protocol Spine (Pairing + QUIC Tunnel).
  - mDNS service discovery on LAN (`_hyperlink._udp.`).
  - TOFU mutual pairing using SHA-256 certificate fingerprints and a symmetric 6-digit PIN.
  - Quinn QUIC server on Linux and client on Android via a Rust JNI bridge (`libhyperlink_bridge.so`).
- **Phase 2 in progress**: Video Pipeline & Protocol Simulation Harness.
  - H.264 video framing protocol (`VideoFrameHeader` with fragmentation, `VideoConfig`, `is_frame_stale`).
  - Android companion `ScreenCaptureService` (`MediaProjection` -> hardware `MediaCodec` H.264 baseline encoder).
  - Linux GStreamer decode pipeline (`appsrc` -> `h264parse` -> `vaapidecodebin`/`avdec_h264` -> `gtk4paintablesink`) and a Libadwaita window with live FPS/bitrate stats.
  - Real-time rolling FPS and bitrate tracking on the host UI (the latency stat is deferred until a calibrated clock-sync offset is wired).
  - Phase 2 video benchmarking harness (`hyperlink-bench video`) measuring transit latency percentiles, effective FPS, and degradation under simulated network loss.
  - Status note: all numbers produced by `hyperlink-bench video` are from a protocol-logic / synthetic streaming simulation, not measured USB/WiFi glass-to-glass latency across physical devices. The physical hardware measurement `docs/SYSTEM_DESIGN.md`'s Phase 2 Definition of Done requires has not happened yet and remains required before Phase 2 can be marked complete.
- **Phase 3 in progress**: Input Injection & Latency Harness.
  - Wire protocol types (`PointerEvent`, `KeyEvent`, `ScrollEvent`, `NavEvent`, `InputAck`) with fixed compact framing and normalized coordinates.
  - Linux GTK4 event controllers (`GestureClick`, `EventControllerMotion`, `EventControllerScroll`, `EventControllerKey`) with aspect-ratio-aware coordinate normalization.
  - Incoming `VideoFrameHeader` width/height wired into `set_video_dimensions`, eliminating a static 1080x1920 assumption and giving pixel-accurate normalized coordinates across arbitrary phone resolutions.
  - Fixed default cargo compilation (`cargo build --workspace`) by feature-gating `async_channel` channel statics and tasks behind `video`.
  - Reliable QUIC bidirectional input stream (type `0x40`) between the Linux host and Android companion.
  - Android companion `InputService` (`AccessibilityService`) for zero-root touch tap injection, swipe/drag gesture tracking (`injectSwipe` on touch motion exceeding tap slop), and system navigation (`Back`, `Home`, `Recents`, `VolumeUp`, `VolumeDown` via `AudioManager`).
  - Android companion text/key injection for focused editable fields via `AccessibilityNodeInfo` (`ACTION_SET_TEXT`), supporting printable characters and Backspace.
  - Phase 3 input latency benchmarking harness (`hyperlink-bench input`) tracking round-trip latency percentiles and packet delivery reliability with a strict <1% loss threshold.
  - Known gaps:
    - *Hardware key injection*: `AccessibilityService` text injection works for focused text inputs, but arbitrary system-wide hardware `KeyEvent` injection outside editable fields (e.g. non-text games, global shortcuts) on unrooted Android over Wi-Fi requires either `INJECT_EVENTS` permission granted via ADB or USB AOA HID emulation, as specified in `docs/SYSTEM_DESIGN.md`.
    - *Hardware DoD validation pending*: a measured physical round-trip button-click test and a 5-minute typing test on physical Android hardware have not yet been performed.
- **Phase 4 in progress**: Notifications Sync & DND Control.
  - Wire protocol types (`NotificationPost`, `NotificationDismiss`, `NotificationActionInvoke`, `DndSync`, `NotificationAck`) with bounded length constraints and a 32 KB icon ceiling to prevent stream saturation.
  - Android companion `NotificationService` (`NotificationListenerService`) capturing posted notifications, titles, body text, action intents, compressed icons, and dismissals.
  - Filtered ongoing / foreground service notifications (`FLAG_ONGOING_EVENT` and `FLAG_FOREGROUND_SERVICE`), so background download meters and media playback controls don't spam host alerts.
  - Bidirectional DND state synchronization (`NotificationManager.INTERRUPTION_FILTER_PRIORITY` <-> `INTERRUPTION_FILTER_ALL`) with desktop alert suppression and header-bar toggle state sync.
  - Fixed host-to-phone notification control routing on stream `0x40` in `android-bridge`, so action invocation, dismissals, and DND toggling from Linux to Android actually work.
  - Fixed keyboard injection keysym vs. scancode: `video_window.rs` now transmits semantic keysym / Unicode codepoints (`keyval.to_unicode()`) rather than raw X11 hardware scancodes, and `InputService.kt` handles Enter/Return (`0xff0d`, `66`), Backspace (`0xff08`, `67`), and full Unicode codepoints.
  - Linux host desktop notification fallback (`dispatch_desktop_notification` via `notify-send`) and Libadwaita in-app notification toasts (`adw::ToastOverlay`, `adw::Toast`) with click-through mirror window activation (`window.present()`), interactive action execution, and dismissal syncing.
  - Phase 4 notification benchmarking harness (`hyperlink-bench notification`) evaluating propagation latency, round-trip action invocation, and delivery loss gating (<0.1% loss, p95 <= 100ms).
  - Known gaps:
    - *Bench simulation vs. hardware*: `hyperlink-bench notification` validates the framing, parsing, and ACK state machine. Physical validation of real-world app notifications (e.g. WhatsApp, Messages) and clicking the banner on Linux to trigger on-device actions on a physical Android phone remains required for final DoD sign-off.
    - *Android notification access*: the companion app requires manual user permission under Settings -> Special App Access -> Notification Access (`BIND_NOTIFICATION_LISTENER_SERVICE`) and Do Not Disturb Access (`ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS`).
- **Phase 5 in progress**: Clipboard Sync & QUIC Stream Multiplexing.
  - Wire protocol types (`ClipboardType`, `ClipboardMessage`, `ClipboardAck`) supporting text (`text/plain`, `text/html`) and images (`image/png`, `image/jpeg`) bounded to 16 MB.
  - Dedicated QUIC stream `0x70` for clipboard: isolates bulk payloads from input (`0x40`) and video (`0x30`) datagrams, avoiding head-of-line blocking during multi-megabyte transfers.
  - Two-tier echo-loop prevention: origin ID tagging (`host:<hostname>` vs `phone:<android_id>`) plus SHA-256 content-hash deduplication (`lastSyncedHash`).
  - Android companion `ClipboardService` (`OnPrimaryClipChangedListener` plus an `InputService` accessibility hook for the Android 10+ background-read workaround) and remote-write application via `ClipboardManager.setPrimaryClip` for text and image `Uri`s.
  - Linux host `ClipboardManager`: background clipboard watcher and remote writer supporting Wayland (`wl-paste`/`wl-copy`) and X11 (`xclip`).
  - Phase 5 clipboard benchmarking harness (`hyperlink-bench clipboard`) evaluating text sync, multi-megabyte image transfers, loop suppression, and concurrent input multiplexing, gating on p95 <= 100ms, zero infinite loops, and input p95 <= 50ms.
  - Known gaps:
    - *Bench simulation vs. physical hardware*: `hyperlink-bench clipboard` validates protocol framing, SHA-256 loop suppression, and transport multiplexing. Physical verification of cross-device copy/paste on a real Android device (copy text on Linux, paste on phone; copy screenshot on phone, paste on Linux) remains required for final physical DoD sign-off.
    - *Android 10+ background restriction*: background apps can't poll `ClipboardManager.getPrimaryClip()` directly without input focus. The app relies on active mirroring / foreground status, or accessibility-anchored text selection events in `InputService`, to read clips copied outside HyperLink.
- **Phase 6 in progress**: File Access (Lazy Virtual Mount).
  - Wire protocol types (`FileEntry`, `FileStatRequest`/`Response`, `FileListRequest`/`Response`, `FileReadChunkRequest`/`Response`, `FileWriteChunkRequest`/`Response`, `FileThumbnailRequest`/`Response`) with a 1 MB max chunk size and a 10,000-entry directory listing ceiling.
  - Dedicated QUIC stream `0x80` for file access: isolates file I/O from input (`0x40`), clipboard (`0x70`), and video datagrams, enabling concurrent file browsing during active mirroring sessions.
  - Android companion `FileAccessService` with runtime storage permission management (`MANAGE_EXTERNAL_STORAGE` for Android 11+, `READ/WRITE_EXTERNAL_STORAGE` for older) and a permission button in `MainActivity`.
  - Android bridge `file_provider` module: POSIX server handlers for `FileStat`, `FileList`, `FileReadChunk` (lazy `SeekFrom::Start` for zero-overhead random access), `FileWriteChunk`, and `FileThumbnail`, rooted at `/storage/emulated/0` with an app-sandbox fallback.
  - Linux host FUSE virtual filesystem (`fuser` 0.18): `HyperLinkFuse` implementing `lookup`, `getattr`, `readdir`, `read`, `write` against the kernel `/dev/fuse` ABI without requiring `libfuse3-dev` C headers.
  - LRU `FileChunkCache` (128 KB chunks, 1024-entry / 128 MB ceiling) with path-level invalidation on write-back, giving sub-millisecond cached reads for active file regions.
  - VFS QUIC client (`VfsClient`) performing request/response round-trips over stream `0x80` for stat, list, read-chunk, write-chunk, and thumbnail operations.
  - Auto-mount at `$XDG_RUNTIME_DIR/hyperlink` (or `~/HyperLink` fallback) on connection, with `AutoUnmount` for clean teardown.
  - Phase 6 file access benchmarking harness (`hyperlink-bench file`) evaluating lazy random-access scrubbing (TTFB p95), write-back verification, and byte-for-byte SHA-256 checksum integrity, gating on TTFB p95 <= 100ms, verified lazy seek (no full download), write-back success, and a 100% checksum match.
  - Known gaps:
    - *Bench simulation vs. physical hardware*: `hyperlink-bench file` validates the protocol framing, chunked read/write round-trip, and checksum integrity via synthetic simulation. Physical verification of browsing photos in a file manager and scrubbing a video in a media player over the mounted FUSE directory on a real Android device remains required for final DoD sign-off.
    - *FUSE privileges*: mounting requires `/dev/fuse` access (`crw-rw-rw-`) and `fusermount3` on the host. In unprivileged containers or sandboxes, the mount fails gracefully with a non-fatal warning.
- **Phase 7 in progress**: Network Resilience & Multipath.
  - Wire protocol types (`PathKind`, `PathStatus`, `FailoverReason`, `PathQuality`, `PathProbeRequest`/`Response`, `PathSwitchNotice`/`Ack`, `PathHeartbeat`, `PathHealthReport`) with discriminants `0x90..=0x95`.
  - QUIC connection migration: `server_config.migration(true)` enabled on the host server and dynamic endpoint rebinding in the client companion, for network handover across interfaces without re-pairing.
  - Android companion `NetworkMonitorService`: registers `ConnectivityManager.NetworkCallback` to detect Wi-Fi, Cellular, and Ethernet transitions and bridges availability to JNI.
  - Android bridge `AndroidMultipathManager`: path candidate evaluation, RTT/jitter probing, automatic standby path selection, and JNI bindings (`onNetworkAvailable`, `onNetworkLost`, `triggerFailover`, `getActivePath`).
  - Linux host `HostMultipathManager`: evaluates active path health, detects packet loss/timeout anomalies, and can trigger failover notices across the control stream (`0x50`).
  - Phase 7 network resilience benchmarking harness (`hyperlink-bench resilience`): simulates mid-session AP disruption (100% link loss), verifies zero re-pairing (session epoch progression and cryptographic token retention), and tracks end-to-end failover detection/switch latency (p95 <= 1000ms, zero input loss, video stream survival).
  - Known gaps:
    - *`HostMultipathManager` is never invoked*: the decision logic (`should_failover`, `initiate_failover`) is implemented and unit-tested, but nothing in `connection.rs`/`main.rs` instantiates or calls it, and Android's `triggerFailover` is likewise never called automatically. Automatic failover does not currently run in either binary — only the QUIC-level `migration(true)` and manual/JNI-triggered paths are live.
    - *The resilience bench is deterministic, not randomized*: unlike the other bench subcommands, `resilience_bench.rs` uses fixed formulas rather than an RNG-driven loss model, so its DoD gate can't currently fail regardless of implementation quality. See `CHANGELOG.md`'s own review notes above for the full finding.
- **Phase 8 in progress**: Proximity & Pre-Warmed Connect.
  - Wire protocol types (`ProximityTechnology`, `ProximityBeacon`, `ProximityAck`, `PreWarmState`, `WorkflowState`, `WorkflowStateAck`) with discriminants `0xA0..=0xA5`.
  - BLE / UWB / Wi-Fi RTT proximity ranging signals intended to pre-warm the QUIC handshake and stream pool before the user launches the mirror.
  - Zero-trust invariant: proximity shortens setup latency but never bypasses certificate-based mTLS authentication against pinned peer fingerprints in the trusted device store.
  - Linux host `HostProximityManager`: tracks a proximity state machine (`Idle`, `BeaconDetected`, `PreWarming`, `PreWarmedReady`, `ActiveMirroring`), handles pre-warm negotiation, and persists workflow state to `~/.config/hyperlink/workflow_state.json`.
  - Workflow state restore: restores mirror window geometry (x, y, width, height, fullscreen state), display orientation, and active Android package name on connect, and persists geometry changes on close.
  - Android companion `ProximityRangingService` and JNI bindings (`sendProximityBeacon`, `saveWorkflowState`, `restoreWorkflowState` in `QuicClient`).
  - Phase 8 proximity benchmarking harness (`hyperlink-bench proximity`): measures cold-connect vs. pre-warmed-connect latency, latency reduction (>50%), time from in-range to usable mirror, workflow state restore fidelity, and untrusted-beacon rejection.
  - Known gaps:
    - *Pre-warming is architecturally circular*: `HostProximityManager` is only reachable from inside `handle_incoming_connection` on the host's control-plane stream — i.e. only after the full mTLS handshake it's meant to precede has already completed. `ProximityRangingService.kt` also fabricates ranging data (`Random.nextInt`) rather than doing real BLE/UWB/Wi-Fi RTT ranging. The zero-trust invariant above holds, but only because there's no real pre-auth path being exercised — the feature can't currently deliver the setup-latency win it's designed for. The bench harness's ">50% latency reduction" is computed from hardcoded idealized ranges, not the real (non-functional) wiring.
- **Phase 9 in progress**: Scoped App-State Handoff.
  - Wire protocol types (`HandoffType`, `BrowserTabState`, `NoteDraftState`, `MediaPlaybackState`, `HandoffPayload`, `HandoffAck`, `HandoffCapability`) with discriminants `0xB0..=0xB3`.
  - Published an open developer specification, `docs/HANDOFF_SPEC.md`, covering URI schemes (`hyperlink-handoff://`), JSON schemas, Android intent filters, and Linux desktop MIME associations for third-party app adoption.
  - Linux host `HostHandoffManager`: auto-launches web browser URLs, saves and opens Markdown note drafts in `~/.local/share/hyperlink/notes/`, and routes media/custom URIs via `xdg-open`.
  - Android companion `HandoffService`: launches Android `Intent.ACTION_VIEW`/`Intent.ACTION_SEND` and bridges outgoing share actions to the Linux peer.
  - Phase 9 handoff benchmarking harness (`hyperlink-bench handoff`): verifies browser tab, note draft, and media playback handoff latency percentiles (p95 <= 100ms), full state restoration fidelity, and schema validation.
  - Fixed (Phase 11): `launch_media`/`launch_uri` in `linux/src/handoff.rs` handed peer-controlled URIs straight to `xdg-open` with no scheme check — a malicious or compromised paired phone could have made the host open arbitrary local files (`file://...`) or an unregistered URI handler. All four launch sites now share one allowlist (`http`, `https`, `hyperlink-note`, `hyperlink-media`, `hyperlink-handoff`).
  - Known gaps:
    - *Bench simulation vs. physical hardware*: `hyperlink-bench handoff` verifies wire serialization, transit, JSON deserialization, and state fidelity. Real-world end-to-end verification of tapping "Continue on Linux PC" in a browser on a physical Android phone and opening the exact tab on desktop remains required for final physical DoD sign-off.
- **Phase 10 in progress**: Ambient Context Agent.
  - Wire protocol types (`AmbientEventCategory`, `AmbientEvent`, `AgentConsentPolicy`, `AgentQueryRequest`, `AgentQueryResponse`) with discriminants `0xC0..=0xC5`.
  - Published an open developer specification, `docs/AMBIENT_AGENT_SPEC.md`, covering telemetry categories, consent models, and the agent query API for third-party AI integration (LangGraph, local LLMs).
  - Linux host `AmbientEventBus`: a local event ring buffer plus disk journal (`~/.local/share/hyperlink/ambient_events.jsonl`) with real-time broadcast and category filtering.
  - Linux host `AmbientContextAgent`: a deterministic query engine answering natural-language queries ("what happened on my phone in the last hour?") with structured activity timelines and citations.
  - Privacy invariants, verified by tracing the actual filter code rather than trusting it exists: raw video is sandboxed (`allow_raw_video: false` by default — true today because no event category carries video frames at all, not because of an active filter with something to block yet); per-category consent gating is enforced at the actual query boundary (`AgentConsentPolicy::can_observe` inside `AmbientEventBus::query`), and `Clipboard` is blocked by default.
  - Android companion `AmbientContextProvider`: background telemetry provider monitoring screen power state, battery charging, and media status, publishing to the JNI bridge's `publishAmbientEvent`.
  - Phase 10 ambient benchmarking harness (`hyperlink-bench ambient`): verifies event publishing latency (p95 <= 10ms), consent gating, raw-video sandboxing, and query synthesis fidelity.
- **Phase 11 in progress**: Production Hardening.
  - Fixed pre-existing Android build breakage: the companion app did not compile before this pass — `ClipboardService` referenced `androidx.core.content.FileProvider` with no `androidx.core` dependency, manifest `<provider>`, or `res/xml/file_paths.xml`, and `MainActivity` called an undefined `createSecondaryButton` helper and a misspelled `logMessage` instead of `log`. Both `./gradlew assembleDebug` and `assembleRelease` now succeed, verified by actually running them.
  - Fixed missing service manifest registrations: `ProximityRangingService` (Phase 8), `HandoffService` (Phase 9), and `AmbientContextProvider` (Phase 10) were real `Service()` subclasses never declared in `AndroidManifest.xml` — undeclared components can't be started or bound on a real device, so those three phases could not run at all regardless of their Kotlin code being correct. Declared now, and wired into `MainActivity` (see below).
  - Linux packaging: a systemd user service unit (`linux/packaging/systemd/hyperlink.service`) and a Flatpak manifest plus desktop entry and AppStream metadata (`linux/packaging/flatpak/`). The Flatpak manifest documents the offline cargo-vendoring step it needs and has not been build-verified (no `flatpak-builder` available in this environment).
  - Android release signing scaffold: `signingConfigs`/`buildTypes` reading a git-ignored `keystore.properties` (template at `android/keystore.properties.example`), R8 minification and resource shrinking enabled for `release`, with explicit keep rules for JNI-bound classes/native methods, verified against the actual R8 `mapping.txt` output — `QuicClient`/`NetworkMonitorService` and their native methods come out unrenamed. No real signing key exists yet, so `assembleRelease` currently falls back to debug signing with a build-time warning rather than silently producing something that looks distributable.
  - Local, privacy-respecting, opt-in crash reporting on both sides (`linux/src/crash_report.rs`, Android `CrashReporter.kt` plus `HyperLinkApplication`): panic/uncaught-exception message, code location, and an optional backtrace only, never clipboard text, notification bodies, file names, or key material, and never transmitted anywhere. Wired into the preferences window's Privacy & Diagnostics page (view/clear reports, toggle on/off).
  - Update check (`linux/src/update_check.rs`): checks a version feed and notifies, but deliberately does not auto-download or auto-install anything — there's no code-signing/release-integrity infrastructure yet to do that safely without introducing a supply-chain risk.
  - Libadwaita preferences window (`linux/src/preferences_window.rs`): feature toggles, video bitrate cap, crash-report/update-check controls, and trusted-device management (view and revoke), all persisted to `DeviceConfig` (extended with a `#[serde(default)]`-gated `HostPreferences` so pre-Phase-11 config files still load). It reads/writes the config file directly rather than sharing live state with an already-running connection — feature/bitrate changes apply next session, and revoking a device does not forcibly disconnect one already connected; both are stated directly in the window's UI copy.
  - Security review of the pairing/auth flow (`docs/SECURITY_REVIEW.md`): a code-level review, not a substitute for the third-party pen-test the DoD calls for, tracing the PIN-confirmation gate end to end rather than assuming it works. One finding fixed here — the pairing window used to stay open (accept-any-cert) for the entire process lifetime; it now closes after one resolved attempt or a 5-minute timeout, without touching `protocol/src/crypto.rs`'s verifier or its handshake test. Two findings remain as documented recommendations: requiring the PIN to be typed rather than eyeballed, and forcing a live disconnect on trust revocation.
  - Onboarding walkthrough (`docs/ONBOARDING.md`): every Linux build/pairing command in it was actually run against this repo. The Android permission grants remain manual taps through Android's own Settings screens (not `adb`) — the DoD's "zero manual dev-tool steps" is met in the sense of no `adb`/CLI steps, but there's no streamlined first-run wizard yet.
  - Wired `ProximityRangingService`/`HandoffService`/`AmbientContextProvider` into `MainActivity`: bound (not just manifest-declared) on `onCreate`, unbound on `onDestroy`. Proximity ranging starts on `onConnected()` and stops on `onDisconnected()`, using a new `QuicClient.ownFingerprint()` JNI getter (`getOwnFingerprint` in `android-bridge`) — `handle_proximity_beacon` on the host matches a beacon's fingerprint prefix against the phone's own pinned identity, not the host's, so the phone needed a way to read its own fingerprint that didn't previously exist.
  - Fixed a deeper gap found while wiring handoff: `QuicClient.EventListener` already declared `onHandoffReceived`/`onAgentConsentUpdated`, but `android-bridge`'s incoming control-stream reader had no decode arm for `MessageType::HandoffPayload` or `AgentConsentUpdate` — both fell into the generic catch-all, so those callbacks could never have fired regardless of what `MainActivity` did with them. Added the corresponding `ClientEvent` variants, their decode arms, and JSON serialization (with proper escaping for the free-form `title`/`uri`/`state_json` fields — the pre-existing arms for fixed-shape fields like `mime_type` interpolate unescaped, a known and separate issue in that older code, not repeated here). `MainActivity.onHandoffReceived` now routes to `HandoffService.handleIncomingHandoff`; `onAgentConsentUpdated` logs, since there's no local Android-side policy store to apply it to yet (enforcement is host-side, in `linux/src/ambient.rs`).
  - Known gaps:
    - *Signed release / store listing*: no real keystore, no store listing, no internal test track — sideload-only today.
    - *Flatpak*: written, not build-verified.
    - *Clean-hardware test*: none of this was run on an actual clean Linux install or factory-reset phone, only in this repo's existing environment.
    - *Third-party pen-test*: not performed; `docs/SECURITY_REVIEW.md` is a starting point for one, not a replacement.
    - *Outgoing handoff / ambient telemetry UI*: `HandoffService.sendHandoffToHost` (phone to host) and manual ambient-telemetry controls have no UI entry point yet — only the receiving direction was reachable to verify end to end this pass.

## [0.1.0] - 2026-07-07
### Added
- **Phase 0 complete**: Foundations and Standalone Measurement Harness.
- Protocol: base wire header (10 bytes) with magic (`b"HLNK"`), version byte (`1`), and message discriminants.
- Clock sync: NTP-style clock offset estimation and linear-regression-based drift measurement.
- Echo tool: latency measurement tracking min/max/mean/p50/p95/p99 and packet loss.
- Transport: a generic `Transport` trait allowing a future transition to QUIC, with a concrete `UdpTransport` implementation for Phase 0.
- Reporting: structured JSON reporting plus human-readable terminal output.

See `docs/SYSTEM_DESIGN.md` for the full phase breakdown.
