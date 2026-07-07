# Project HyperLink — System Design & Phased Build Plan
### Linux ⇄ Samsung Ultra-Low-Latency Device Union
Spec version: 0.1 — Author: Arjun — For: coding agent execution

---

## 0. How To Use This Document

This is a **phase-gated** build plan, not a sprint calendar. Each phase has a hard **Definition of Done (DoD)**. Do not start phase N+1 until phase N's DoD is met and verified with real measurements, not assumptions. If a phase's DoD can't be met as specified, stop and flag it — don't silently degrade scope.

Every phase after Phase 0 must report its numbers against the bench harness built in Phase 0. "It feels fast" is not an acceptable DoD anywhere in this document.

---

## 1. Product Definition

**One-line vision:** HyperLink turns a Samsung phone and a Linux desktop into one continuous compute surface — mirrored display, shared input, shared clipboard/notifications/files, and (later) shared compute — over a single custom low-latency link, without depending on Microsoft or Samsung's closed protocols.

### Explicit Non-Goals (scope discipline — read this before writing any code)
These were tempting in early brainstorming and are **deliberately excluded** because they are not honestly buildable as first described:
- ❌ Raw GPU framebuffer access on Android — not exposed to third-party apps at any privilege level below custom firmware. We use MediaProjection + hardware encode instead.
- ❌ Kernel-level "sub-1ms" bypass without root/custom firmware — not available on stock Samsung devices. We optimize within Android's real APIs.
- ❌ Full Samsung DeX-style third-party app windowing — relies on undocumented OEM hooks. Out of scope permanently unless Samsung publishes an SDK.
- ❌ Universal app-state handoff for arbitrary third-party apps — requires their cooperation (same limit Apple's Handoff has). We support our own surfaces + a documented opt-in contract, not magic universal support.
- ❌ Skipping real authentication because a device is "nearby" (UWB proximity) — proximity may *pre-warm* a connection, never *substitute for* a cryptographic auth check.

### Target Hardware Assumptions
- Linux desktop/laptop: modern GPU with VAAPI or NVDEC (Intel/AMD/NVIDIA), USB-C, WiFi 6E/7 recommended but not required
- Samsung phone: Android 12+, USB-C, WiFi 6/6E/7 recommended, UWB optional (Tier 2 feature only)

---

## 2. Architecture Overview

```
┌─────────────────────────┐        HyperLink Protocol        ┌──────────────────────────┐
│   HyperLink-android        │◄══════ single QUIC tunnel ══════►│   HyperLink-linux           │
│   (Kotlin service)        │   multiplexed streams:          │   (Rust + GTK4/Libadwaita)│
│                           │   - video (unreliable)          │                           │
│  MediaProjection→Codec    │   - input (reliable, small)     │  GStreamer decode→paintable│
│  NotificationListener     │   - control-plane (notif/clip)  │  Event controllers→input  │
│  Clipboard access-svc     │   - file (reliable, chunked)    │  FUSE virtual mount        │
│  FUSE-servable file API   │                                 │  Libadwaita settings UI    │
└─────────────────────────┘                                 └──────────────────────────┘
             ▲                                                            ▲
             └───────────────────── HyperLink-bench (latency/throughput harness, built first) ──────┘
```

### Components (monorepo layout)
```
HyperLink/
├── protocol/         # shared schema, versioned, source of truth for both sides
├── android/          # Kotlin companion service
├── linux/            # Rust host application
├── bench/            # measurement harness — built in Phase 0, used every phase after
└── docs/             # this file + decision log + threat model
```

---

## 3. Key Technical Decisions (with rationale)

| Decision | Choice | Why |
|---|---|---|
| Transport | QUIC (TLS 1.3 built-in) | Stream multiplexing without head-of-line blocking; one big file transfer won't stall video frames |
| Linux language | Rust | GC pauses / GIL stalls directly ruin tail latency; this is the one place it's non-negotiable |
| Linux UI | GTK4 + Libadwaita | Matches your existing Glass Stickies design language; native paintable sink for GStreamer |
| Android language | Kotlin | Standard, full API access to MediaProjection/MediaCodec/NotificationListenerService |
| Video codec | H.264 baseline (HEVC/AV1 as later optimization) | Hardware encode/decode ubiquity now; revisit AV1 once H.264 path is proven |
| Serialization | FlatBuffers (video/input path), Protobuf (control-plane) | Zero-copy where latency matters; protobuf's ergonomics fine for a notification payload |
| Pairing | QR/PIN exchange → cert fingerprint, Trust-On-First-Use | Same trust model as Signal/KDE Connect; no cloud account dependency |
| Video reliability | Unreliable/best-effort stream, drop stale frames | Freshness beats completeness for a live mirror |
| Input reliability | Reliable stream, small payloads | A dropped click is worse than a delayed one |

---

## 4. Phase-by-Phase Plan

### Phase 0 — Foundations & Measurement Harness
**Why first:** you cannot optimize a link you can't measure, and every later phase's DoD depends on this existing.

Tasks:
- Set up monorepo structure (`protocol/`, `android/`, `linux/`, `bench/`, `docs/`)
- Define protocol versioning strategy (a version byte in the handshake, from day one — retrofitting this later is painful)
- Build `samyoga-bench`: a clock-sync handshake (NTP-style round-trip offset calculation) between phone and host, since cross-device latency measurement is meaningless without synchronized clocks
- Build a minimal echo tool: timestamp → send → timestamp on receipt → report round-trip and estimated one-way latency
- Define and document target metrics table (fill in real numbers once Phase 2/3 give you a baseline):
  - Video glass-to-glass latency (USB / 5GHz WiFi / 6GHz WiFi)
  - Input round-trip latency
  - Notification propagation latency
  - Clipboard sync latency
  - File throughput (MB/s) and time-to-first-byte for lazy reads

**DoD:** clock sync accuracy verified under 1ms drift over a 60-second window; echo tool produces a logged, repeatable latency number on a real USB and real WiFi connection between your actual devices.

---

### Phase 1 — Protocol Spine (Pairing + QUIC Tunnel)
Tasks:
- mDNS/avahi discovery on LAN
- QR-code or PIN-based first-pairing flow, exchanging a cert fingerprint (TOFU)
- QUIC connection establishment (server on Linux host, client role on Android, or vice versa — decide and document which side listens)
- Stream multiplexing scaffold: define stream type IDs for video/input/control/file up front, even before those features exist
- Reconnect/resume logic — network drop should not require re-pairing
- Versioned handshake so future protocol changes don't hard-break older builds

**DoD:** two minimal processes (real phone + real Linux host, no emulators) pair once, survive a WiFi toggle-off/on with automatic reconnect, exchange a heartbeat on the control stream, and a packet capture confirms only TLS records are visible on the wire (no plaintext leakage).

---

### Phase 2 — One-Way Video Mirroring
Tasks:
- Android: MediaProjection capture → MediaCodec H.264 hardware encoder configured for low latency (zero B-frames, short GOP, CBR or capped VBR)
- Chunk encoded frames onto the video stream (unreliable, drop-if-stale policy)
- Linux: GStreamer pipeline, hardware decode (VAAPI/NVDEC depending on your GPU), render into a GTK4 paintable widget
- Adaptive bitrate/resolution driven by live measurements from `samyoga-bench` (don't hardcode a bitrate — react to observed loss/jitter)

**DoD:** measured glass-to-glass latency logged for USB and WiFi paths against concrete targets (suggested starting targets: <60ms USB, <100ms 5GHz WiFi — adjust once you have real hardware numbers). Frame drops under induced packet loss (test with `tc/netem`) degrade smoothly, no freeze/crash.

---

### Phase 3 — Input Injection (Closing the Loop)
Tasks:
- Linux: GTK4 event controllers capture mouse/keyboard/touch, normalize coordinates against the mirrored video's actual resolution/orientation
- Serialize onto the reliable input stream
- Android: inject via AOA HID emulation (USB path) or an explicitly user-granted `INJECT_EVENTS`-equivalent permission flow (WiFi path — this requires a one-time ADB grant; document this clearly as a real limitation of unrooted Android, not a bug)
- Handle DPI/orientation/resolution mismatches between phone and rendered window

**DoD:** round-trip test — click a specific on-screen button from the Linux side and confirm the phone-side action fires and the visual feedback returns — measured and logged. A real task (typing a note end-to-end using only PC keyboard) works reliably for a 5-minute session with no missed/duplicated inputs.

---

### Phase 4 — Notifications Sync
Tasks:
- Android: `NotificationListenerService`, structured payload (app id, icon, title, body, actions)
- Control-plane stream delivery
- Linux: native GTK4/Libadwaita notification rendering, click-through action (bring mirror to front / launch relevant app view)
- Do-not-disturb state sync in both directions

**DoD:** notification appears on Linux within a measured, logged delay of it firing on-device; clicking it correctly surfaces the right context on the Linux side.

---

### Phase 5 — Clipboard Sync
Tasks:
- Bidirectional clipboard watchers, each write tagged with an origin ID to prevent ping-pong loops
- Android: background clipboard read workaround (Android 10+ restricts this — you'll need an accessibility service or a foreground-anchored approach; document the exact mechanism you land on, since this is the trickiest permission surface in the whole project)
- Handle large clipboard payloads (images) without blocking the UI thread on either side

**DoD:** copy-paste verified in both directions for text and image content; stress test confirms no infinite sync loop; large image clipboard content doesn't stall input/video streams (proves your multiplexing is actually working).

---

### Phase 6 — File Access (Lazy Virtual Mount)
Tasks:
- Linux: FUSE filesystem exposing phone storage as a mounted directory
- Lazy, chunked reads over a reliable stream — only fetch the bytes actually requested (e.g., video scrubbing shouldn't pull the whole file)
- Write-back support
- Thumbnail/metadata prefetch strategy for gallery-style browsing
- Local caching layer with sane eviction

**DoD:** browse phone photos via a standard Linux file manager with lazy thumbnail loading; open a large video file directly from the mount in a real editor without a full upfront copy; checksum-verify that a fully-read file via the mount matches a direct copy byte-for-byte.

---

### Phase 7 — Network Resilience & Multipath
Tasks:
- WiFi 7 Multi-Link Operation (MLO) where hardware supports bonding 5GHz+6GHz — real 802.11be feature, not the document's "sub-1ms" claim, but genuinely lower jitter
- Custom multi-path scheduler inside SAMYOGA (WiFi + phone cellular tether) — build your own rather than relying on Android's weak native MPTCP support
- Automatic failover specifically prioritized for video/input streams (those can't tolerate a stall the way a file transfer can)
- Feed failover events into the bench harness for visibility

**DoD:** killing the WiFi AP mid-session triggers failover to the tether path within a measured, logged time window, without requiring re-pairing; session state (video/input) survives the transition.

---

### Phase 8 — Proximity & Pre-Warmed Connect
Tasks:
- UWB ranging integration (hardware-dependent) to pre-warm the QUIC handshake before the user consciously initiates a connection
- Explicit note: this shortens *connection setup latency*, it must never bypass the cert-based auth step
- Workflow-state restore hook (reopen last mirrored view) on connect

**DoD:** measured time-from-in-range to usable-mirror; a security review checklist item confirms proximity data is never used as an auth substitute, only as a connection-warming trigger.

---

### Phase 9 — Scoped App-State Handoff
Tasks:
- Define a documented intent/deep-link contract for state serialization
- Implement for SAMYOGA's own companion surfaces first
- Publish the contract as something a third-party app *could* adopt (explicitly not universal — this is the honest version of "handoff")

**DoD:** demonstrate handoff mid-task on at least one real flow (e.g., a note or browser tab) between phone and PC using the documented contract.

---

### Phase 10 — Ambient Context Agent (the actual differentiator)
This is the feature that separates SAMYOGA from being "just another Phone Link clone" — it leans into your multi-agent/agentic background rather than competing purely on mirroring quality.

Tasks:
- Expose an internal event bus (notifications, screen-state metadata, clipboard events) that a desktop agent can subscribe to
- Explicit per-category opt-in/consent gating — this handles personal/sensitive data, treat it accordingly
- Sandbox the agent from raw video by default — metadata only unless the user explicitly grants more
- Reference integration: a demo agent (e.g., using your existing LangGraph stack) that answers "what happened on my phone in the last hour"

**DoD:** a working demo agent consuming the event bus end to end, with consent gating verified to actually block ungranted categories.

---

### Phase 11 — Production Hardening
Tasks:
- Linux: systemd user service + Flatpak packaging
- Android: signed APK (sideload or internal test track), proper foreground-service notification (required for MediaProjection persistence)
- Structured, opt-in logging + crash reporting (privacy-respecting — no raw content in logs)
- Auto-update mechanism for both sides
- Libadwaita preferences window (pairing management, feature toggles, bandwidth limits)
- Threat-model review and a real pen-test pass focused specifically on the pairing/auth flow
- Full onboarding flow documented and tested on a clean machine + clean phone

**DoD:** a fresh install on a clean Linux machine and a clean phone reaches full functionality via the documented setup flow alone, in under a defined time budget, with zero manual `adb`/dev-tool steps required from an end user.

---

## 5. Cross-Cutting Concerns (apply in every phase, not a phase themselves)

- **Security:** cert pinning after first pairing, replay protection on all control-plane messages, no auth-bypass path ever, a documented revocation/re-pair procedure
- **Observability:** structured logs from Phase 0 onward; every phase reports its numbers through `samyoga-bench`, not ad hoc prints
- **Protocol compatibility:** the version byte from Phase 1 must gate every subsequent protocol change
- **Testing:** unit tests per component, plus an integration harness using `tc/netem` to simulate real-world packet loss and latency — test under bad network conditions, not just your desk WiFi

---

## 6. Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| OEM/Android-version permission drift (Samsung One UI restrictions) | Feature breakage across phone models/OS versions | Pin tested Android/One UI versions early, document minimum supported version |
| USB accessory mode inconsistency across cables/ports | Unreliable USB path | Detect and fall back to WiFi gracefully, surface a clear UI warning |
| GPU decode driver variance (Intel/AMD/NVIDIA) | Inconsistent video performance on Linux | Runtime-detect available hardware decode path, software fallback with clear perf warning |
| Battery drain from continuous screen capture | Poor real-world usability | Auto-pause capture when mirror window isn't focused/visible |
| Legal/IP posture | — | This is a clean-room, from-scratch protocol for personal device pairing — it does not reverse-engineer or reimplement Microsoft's or Samsung's proprietary Phone Link protocol; keep it that way throughout |

---

## 7. Suggested Execution Grouping (epics, not calendar dates)

- **Epic A** — Phase 0–1: protocol spine, nothing user-visible yet but everything depends on it
- **Epic B** — Phase 2–3: mirror + control. This alone is already a usable product — a real milestone worth celebrating and dogfooding
- **Epic C** — Phase 4–6: continuity features (notifications/clipboard/files)
- **Epic D** — Phase 7–9: resilience and polish
- **Epic E** — Phase 10–11: the differentiator feature + shippable packaging

---

## 8. "Production Ready" Checklist

- [ ] All phase DoDs met and logged with real measurements
- [ ] Threat model reviewed, pairing/auth flow pen-tested
- [ ] Clean-machine onboarding tested end to end
- [ ] Crash reporting and structured logs in place, privacy-respecting
- [ ] Documented minimum supported Android/One UI and Linux distro/GPU driver versions
- [ ] Auto-update path verified for both Android and Linux builds
- [ ] Bench harness numbers published for all core latency metrics (video, input, notification, clipboard, file)
