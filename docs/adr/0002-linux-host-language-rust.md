# ADR-0002: Linux Host Implementation Language = Rust

## Status
Accepted

## Context
The video/input path directly determines whether the product feels "local." A GC pause or GIL contention in that path is a direct, visible latency spike — not an abstract concern.

## Decision
Implement the entire `linux/` host application in Rust, using `gstreamer-rs` for the decode pipeline and `gtk4-rs`/`libadwaita-rs` for the UI.

## Consequences
Slower initial development velocity than Python, but removes an entire class of latency-tail risk. The QUIC (`quinn`) and GStreamer binding ecosystem in Rust is mature enough that this isn't a bet on unproven tooling.
