# Changelog

All notable changes to this project are documented here. Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-07-07
### Added
- **Phase 0 Complete**: Foundations and Standalone Measurement Harness.
- **Protocol**: Base wire header (10 bytes) with magic (`b"HLNK"`), version byte (`1`), and message discriminants.
- **Clock Sync**: NTP-style clock offset estimation and linear regression-based drift measurement.
- **Echo Tool**: Latency measurement tool tracking Min/Max/Mean/P50/P95/P99 latency percentiles and packet loss.
- **Transport**: Generic `Transport` trait allowing future transition to QUIC with concrete `UdpTransport` implementation for Phase 0.
- **Reporting**: Structured JSON reporting + human-readable terminal output.

See `docs/SYSTEM_DESIGN.md` for the full phase breakdown.
