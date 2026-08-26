# protocol/

Shared wire-format schema and type definitions for HyperLink — the contract both `android/` and `linux/` implement independently.

See `docs/adr/0004-serialization-flatbuffers-protobuf.md` for the serialization rationale and `docs/SYSTEM_DESIGN.md` Phase 1 for the versioning strategy.

**Status:** Message types through Phase 10 (`0x01`-`0xC5`) are defined and round-trip tested. See `CHANGELOG.md` for what's implemented on each side of the wire versus still pending hardware validation.

## Wire Specification

### Base Wire Header (10 Bytes)

Every message transmitted over the wire begins with a fixed-size header:

| Field | Size | Type | Description |
|---|---|---|---|
| Magic | 4 bytes | `[u8; 4]` | Packet identification: `b"HLNK"` |
| Version | 1 byte | `u8` | Protocol version byte (currently `1`) |
| Message Type | 1 byte | `u8` | Message type discriminant |
| Payload Length | 4 bytes | `u32` (BE) | Length of the payload in bytes |

### Implemented Message Types

- **Phase 0 (Bench Harness)**:
  - `ClockSyncRequest` (`0x01`): Initiates NTP-style clock sync, sending client timestamp `t1`.
  - `ClockSyncResponse` (`0x02`): Responds with `t1` (echoed), `t2` (server receive time), and `t3` (server send time).
  - `EchoRequest` (`0x10`): Latency test request with sequence and client send time.
  - `EchoResponse` (`0x11`): Server response echoing request details alongside server receive time.
  - `Heartbeat` (`0x20`): Keepalive message.
- **Phase 1 (Control Plane & Pairing)**:
  - `PairRequest` (`0x50`): Pairing initiation.
  - `PairResponse` (`0x51`): Pairing acknowledgement.
- **Phase 2 (Video Stream)**:
  - `VideoFrame` (`0x30`): Encoded H.264 video frame chunks (`VideoFrameHeader` with fragmentation, timestamp, and keyframe flags).
  - `VideoConfig` (`0x31`): Codec configuration (SPS/PPS, bitrate, FPS).
  - `BitrateAck` (`0x32`): Receiver bitrate feedback.
- **Phase 3 (Input Stream)**:
  - `PointerEvent` (`0x40`): Pointer movement, clicks, and touch events (normalized `0..65535` fixed-point coordinates).
  - `KeyEvent` (`0x41`): Keyboard key press and release events with modifier flags.
  - `ScrollEvent` (`0x42`): Mouse wheel deltas and gesture scroll events.
  - `NavEvent` (`0x43`): System navigation events (Back, Home, Recents).
  - `InputAck` (`0x44`): Input round-trip latency acknowledgement.
- **Phase 4 (Notification Stream)**:
  - `NotificationPost` (`0x60`): Notification posted on device (id, app, title, body, actions, icon bytes).
  - `NotificationDismiss` (`0x61`): Notification dismissed on phone or host.
  - `NotificationActionInvoke` (`0x62`): Notification action button clicked on host.
  - `DndSync` (`0x63`): Do-Not-Disturb state synchronization.
  - `NotificationAck` (`0x64`): Notification transit latency acknowledgement.
- **Phase 5 (Clipboard Stream)**:
  - `ClipboardMessage` (`0x70`): Text or image clipboard payload, tagged with an origin ID and content hash for echo-loop prevention.
  - `ClipboardAck` (`0x71`): Clipboard delivery acknowledgement.
- **Phase 6 (File Access Stream)**:
  - `FileStatRequest` / `FileStatResponse` (`0x80` / `0x81`): Metadata lookup for a single path.
  - `FileListRequest` / `FileListResponse` (`0x82` / `0x83`): Directory listing.
  - `FileReadChunkRequest` / `FileReadChunkResponse` (`0x84` / `0x85`): Lazy chunked read at an arbitrary offset.
  - `FileWriteChunkRequest` / `FileWriteChunkResponse` (`0x86` / `0x87`): Chunked write-back.
  - `FileThumbnailRequest` / `FileThumbnailResponse` (`0x88` / `0x89`): Gallery thumbnail generation.
- **Phase 7 (Network Resilience & Multipath)**:
  - `PathProbeRequest` / `PathProbeResponse` (`0x90` / `0x91`): RTT/jitter probing for a candidate path.
  - `PathSwitchNotice` / `PathSwitchAck` (`0x92` / `0x93`): Failover handoff between paths.
  - `PathHeartbeat` (`0x94`): Liveness check on the active path.
  - `PathHealthReport` (`0x95`): Aggregated path quality summary.
- **Phase 8 (Proximity, Pre-Warming & Workflow State)**:
  - `ProximityBeacon` / `ProximityAck` (`0xA0` / `0xA1`): BLE/UWB/Wi-Fi RTT proximity signal and acceptance/rejection.
  - `PreWarmState` (`0xA2`): Pre-warm handshake/stream-pool status sync.
  - `WorkflowStateSave` / `WorkflowStateRestore` / `WorkflowStateAck` (`0xA3` / `0xA4` / `0xA5`): Window geometry, orientation, and active-app state.
- **Phase 9 (Scoped App-State Handoff)**:
  - `HandoffPayload` / `HandoffAck` (`0xB0` / `0xB1`): Browser tab, note draft, media playback, or custom-URI state transfer.
  - `HandoffDiscover` / `HandoffDiscoverAck` (`0xB2` / `0xB3`): Capability negotiation (supported handoff types/schemes).
- **Phase 10 (Ambient Context Agent)**:
  - `AmbientEventPublish` / `AmbientEventAck` (`0xC0` / `0xC1`): Telemetry event publish and delivery acknowledgement.
  - `AgentConsentUpdate` / `AgentConsentQuery` (`0xC2` / `0xC3`): Per-category consent policy sync.
  - `AgentQueryRequest` / `AgentQueryResponse` (`0xC4` / `0xC5`): Natural-language query against the ambient event bus.
