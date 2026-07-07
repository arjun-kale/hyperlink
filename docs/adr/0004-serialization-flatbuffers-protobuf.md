# ADR-0004: Serialization = FlatBuffers (Hot Path) + Protobuf (Control-Plane)

## Status
Accepted

## Context
Video and input messages are frequent and latency-critical. Notification/clipboard/file-header messages are far less frequent and benefit more from tooling ergonomics than from zero-copy access.

## Decision
Use FlatBuffers for the video and input stream payloads. Use Protobuf for control-plane and file-header messages.

## Consequences
Two serialization toolchains to maintain instead of one — but each is matched to its actual latency budget rather than forcing a single one-size-fits-all format onto both use cases.
