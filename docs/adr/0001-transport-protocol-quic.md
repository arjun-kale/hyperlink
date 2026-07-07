# ADR-0001: Use QUIC as the Transport Protocol

## Status
Accepted

## Context
HyperLink needs multiplexed channels with very different reliability/latency needs — a loss-tolerant video stream, a loss-intolerant small input stream, a reliable control-plane, and a reliable bulk file-transfer stream — over both USB-tunneled and Wi-Fi links, with encryption mandatory on every connection.

## Decision
Use QUIC (RFC 9000), which bundles TLS 1.3 into the transport itself, via the `quinn` crate on the Rust/Linux side.

## Consequences
Gain true stream-level multiplexing without TCP's head-of-line blocking, and encryption is structural rather than a bolted-on layer. Trade-off: more implementation complexity than plain TCP sockets, and congestion control must be tuned carefully so bulk file transfer doesn't starve the latency-critical video/input streams sharing the same connection.
