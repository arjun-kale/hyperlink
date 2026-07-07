# protocol/

Shared wire-format schema and type definitions for HyperLink — the contract both `android/` and `linux/` implement independently.

See `docs/adr/0004-serialization-flatbuffers-protobuf.md` for the serialization rationale and `docs/SYSTEM_DESIGN.md` Phase 1 for the versioning strategy.

**Status:** 🚀 Phase 0 foundations implemented.

## Phase 0 Specification

Phase 0 establishes the base protocol header structure and message format for measurement, paving the way for FlatBuffers (video/input) and Protobuf (control/file) serialization in Phase 1.

### Base Wire Header (10 Bytes)

Every message transmitted over the wire begins with a fixed-size header:

| Field | Size | Type | Description |
|---|---|---|---|
| Magic | 4 bytes | `[u8; 4]` | Packet identification: `b"HLNK"` |
| Version | 1 byte | `u8` | Protocol version byte (currently `1`) |
| Message Type | 1 byte | `u8` | Message type discriminant |
| Payload Length | 4 bytes | `u32` (BE) | Length of the payload in bytes |

### Phase 0 Message Types

- **`ClockSyncRequest` (`0x01`)**: Initiates NTP-style clock sync, sending client timestamp `t1`.
- **`ClockSyncResponse` (`0x02`)**: Responds with `t1` (echoed), `t2` (server receive time), and `t3` (server send time).
- **`EchoRequest` (`0x10`)**: Latency test request with sequence and client send time.
- **`EchoResponse` (`0x11`)**: Server response echoing request details alongside server receive time.
- **`Heartbeat` (`0x20`)**: Keepalive message.
