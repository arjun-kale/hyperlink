# HyperLink

**Linux ⇄ Samsung, one continuous compute surface.**

A from-scratch, low-latency device-union protocol — mirrored display, shared input, notifications, clipboard, and files between a Linux host and a Samsung Android device over a single multiplexed QUIC tunnel. Built as a clean-room alternative to closed device-linking protocols, not a reverse-engineering of them.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
![Status](https://img.shields.io/badge/status-architecture%20%2F%20design-yellow)
![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20Android-informational)
[![CI](https://github.com/<your-username>/HyperLink/actions/workflows/ci.yml/badge.svg)](https://github.com/<your-username>/HyperLink/actions/workflows/ci.yml)

## Why

Existing solutions either compromise on latency (standard screen mirroring over Wi-Fi) or require closed, vendor-locked protocols (Microsoft Phone Link, Samsung Link to Windows). HyperLink is a single, versioned, self-hosted protocol — no cloud account, no vendor lock-in — built around one goal: make the boundary between your phone and your Linux desktop disappear, honestly, within what stock hardware actually allows.

## Architecture

```mermaid
flowchart LR
    subgraph Phone["HyperLink-android (Kotlin)"]
        A1[MediaProjection + MediaCodec H.264]
        A2[NotificationListenerService]
        A3[Clipboard access service]
        A4[FUSE-servable file API]
    end

    subgraph Tunnel["HyperLink Protocol — single QUIC tunnel, TLS 1.3"]
        S1[video stream — unreliable]
        S2[input stream — reliable]
        S3[control-plane stream — notif/clipboard]
        S4[file stream — reliable, chunked]
    end

    subgraph Host["HyperLink-linux (Rust + GTK4/Libadwaita)"]
        H1[GStreamer decode → paintable]
        H2[Event controllers → input]
        H3[Notification + clipboard sync]
        H4[FUSE virtual mount]
    end

    A1 --> S1 --> H1
    H2 --> S2 --> A1
    A2 --> S3 --> H3
    A3 <--> S3 <--> H3
    A4 --> S4 --> H4

    Bench[HyperLink-bench — latency/throughput harness]
    Bench -.measures.-> S1
    Bench -.measures.-> S2
    Bench -.measures.-> S3
    Bench -.measures.-> S4
```

Full breakdown: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

## Build Status

Full plan with a measured Definition-of-Done per phase: [`docs/SYSTEM_DESIGN.md`](docs/SYSTEM_DESIGN.md)

## Tech Stack

| Layer | Choice | Rationale |
|---|---|---|
| Transport | QUIC / TLS 1.3 | Multiplexed streams, no head-of-line blocking |
| Linux host | Rust, GTK4, Libadwaita | Tail latency matters; no GC pauses |
| Android companion | Kotlin | Full `MediaProjection`/`MediaCodec`/`NotificationListener` access |
| Hot-path serialization | FlatBuffers | Zero-copy for video/input |
| Control-plane serialization | Protobuf | Ergonomics over raw throughput |
| Pairing | TOFU cert fingerprint | No cloud account dependency |

Decision rationale for each of these: [`docs/adr/`](docs/adr/)

## Security

Threat model tracked from day one, not bolted on at the end: [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md)

## Repository Structure

```
HyperLink/
├── protocol/     # shared wire schema — source of truth for both sides
├── android/      # Kotlin companion service
├── linux/        # Rust host application
├── bench/        # latency/throughput measurement harness
└── docs/         # system design, architecture, ADRs, threat model
```

## Status

This project is in the design/architecture phase. Implementation follows the phased plan in `docs/SYSTEM_DESIGN.md`, gated by measured Definition-of-Done criteria — no phase ships on "it feels fast," only on logged numbers from `bench/`.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Apache License 2.0 — see [`LICENSE`](LICENSE).
