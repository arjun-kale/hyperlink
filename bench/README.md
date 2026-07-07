# bench/

Standalone latency and throughput measurement harness. Built first (Phase 0) and used to validate the Definition-of-Done (DoD) for every phase after it. It is a development tool that every other component's numbers are measured against.

**Status:** 🚀 Phase 0 foundations implemented.

## Features

- **Clock Synchronization**: NTP-style clock sync estimating the one-way offset and tracking drift rate over a window (DoD requires drift under 1ms/60s). Uses linear regression for drift assessment.
- **Echo Tool**: Measures RTT latency with statistics aggregation (Min, Max, Mean, Median, p55, p95, p99, StdDev, Loss).
- **Transport Abstraction**: Built around a generic `Transport` trait, allowing seamless transition from UDP sockets to QUIC tunnel streams in Phase 1.
- **Structured Reporting**: Produces formatted terminal summaries and appends timestamped reports to `bench/results/` in JSONL format.

## Usage

Ensure your shell has Cargo on the path (`source "$HOME/.cargo/env"` if recently installed).

### 1. Build the Harness
```sh
cargo build -p hyperlink-bench
```

### 2. Start the Server (Responder)
Bind the server to a specific address/port (defaults to `0.0.0.0:9900`):
```sh
cargo run -p hyperlink-bench -- server --bind 127.0.0.1:9900
```

### 3. Run the Client (Initiator)
Connect to the server and initiate the benchmark:
```sh
cargo run -p hyperlink-bench -- client --target 127.0.0.1:9900
```

### Client CLI Options

```sh
# Run with customized sync rounds, duration, and packet count
cargo run -p hyperlink-bench -- client --target 127.0.0.1:9900 \
    --clock-sync-rounds 16 \
    --drift-window 30.0 \
    --echo-count 200 \
    --echo-interval 20

# Output results as raw JSON instead of the terminal table
cargo run -p hyperlink-bench -- client --target 127.0.0.1:9900 --json

# Run with customized payload padding (e.g. 1400 bytes to simulate full packet load)
cargo run -p hyperlink-bench -- client --target 127.0.0.1:9900 --payload-size 1400
```
