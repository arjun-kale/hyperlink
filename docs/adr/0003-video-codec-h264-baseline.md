# ADR-0003: Initial Video Codec = H.264 Baseline (Defer HEVC/AV1)

## Status
Accepted

## Context
The pipeline needs hardware encode on essentially every Samsung device and hardware decode on essentially every Linux GPU vendor (Intel/AMD/NVIDIA) without per-vendor special-casing before the core pipeline is even proven to work.

## Decision
Ship H.264 baseline profile first — zero B-frames, short GOP, low-latency encoder mode. Treat HEVC/AV1 as a later optimization once the core pipeline is validated end to end.

## Consequences
Leaves compression efficiency on the table initially. In exchange, hardware-compatibility risk is removed from the critical path of proving the core mirror-and-control loop works at all.
