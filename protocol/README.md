# protocol/

Shared wire-format schema for HyperLink — the contract both `android/` and `linux/` implement independently. FlatBuffers definitions for the video/input hot path, Protobuf definitions for control-plane messages (notifications/clipboard/file headers).

See `docs/adr/0004-serialization-flatbuffers-protobuf.md` for the rationale and `docs/SYSTEM_DESIGN.md` Phase 1 for the versioning strategy.

**Status:** 📋 not yet implemented — schema design begins in Phase 1.
