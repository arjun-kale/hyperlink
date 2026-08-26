# bench/

Standalone latency and throughput measurement harness. Built first (Phase 0) and used to validate the Definition-of-Done (DoD) for every phase after it. It is a development tool that every other component's numbers are measured against.

**Status:** Phase 0 foundations implemented; `video`, `input`, `notification`, `clipboard`, `file`, `resilience`, `proximity`, `handoff`, and `ambient` subcommands added for later phases. Most of those measure protocol-layer behavior in simulation rather than physical hardware — see each subcommand's note below and `CHANGELOG.md` for exactly what's been hardware-validated.

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

### 4. Run Phase 2 Video Stream Benchmark

Validate Phase 2 video streaming, latency percentiles, and degradation under simulated network loss:

```sh
# Clean baseline (0% loss, 30 fps, 4 Mbps)
cargo run -p hyperlink-bench -- video --frames 300 --fps 30 --bitrate-kbps 4000 --loss-rate 0.0

# Network degradation test (10% packet loss)
cargo run -p hyperlink-bench -- video --frames 300 --fps 30 --bitrate-kbps 4000 --loss-rate 0.10

# Output results as JSON
cargo run -p hyperlink-bench -- video --frames 300 --fps 30 --loss-rate 0.0 --json
```

> **Note**: `hyperlink-bench video` evaluates protocol-layer packetization, fragmentation, stale-frame detection, and degradation behavior in simulation. It does not measure physical glass-to-glass latency (camera/screen photons to Linux display) on real hardware, which remains required for the final Phase 2 DoD sign-off.

### 5. Run Phase 3 Input Latency Benchmark

Measure input event framing, round-trip acknowledgement latency (RTT), and reliability under simulated network conditions:

```sh
# Clean baseline (100 events, 5ms interval, 0% loss)
cargo run -p hyperlink-bench -- input --count 100 --interval-ms 5 --loss-rate 0.0

# Loss degradation test (10% packet loss)
cargo run -p hyperlink-bench -- input --count 100 --interval-ms 5 --loss-rate 0.10

# Output results as JSON
cargo run -p hyperlink-bench -- input --count 100 --json
```

> **Note**: `hyperlink-bench input` evaluates protocol serialization, dispatch, and ack accounting in simulation. Hardware measurement of physical round-trip action feedback on an Android device remains required for the final Phase 3 DoD sign-off.

### 6. Run Phase 4 Notification Sync Benchmark

Measure notification propagation latency (time from post on-device to receipt on host), round-trip interactive action invocation, and packet delivery bounds:

```sh
# Clean baseline (50 notifications, 10 actions, 10ms interval, icons included)
cargo run -p hyperlink-bench -- notification --count 50 --interval-ms 10 --actions 10 --icons

# Loss degradation test (5% packet loss)
cargo run -p hyperlink-bench -- notification --count 50 --interval-ms 0 --loss-rate 0.05

# Output results as JSON
cargo run -p hyperlink-bench -- notification --count 50 --json
```

> **Note**: `hyperlink-bench notification` evaluates protocol serialization, compression, and delivery reliability in simulation. Testing real third-party notifications across physical hardware remains required for the final Phase 4 DoD sign-off.

### 7. Run Phase 5 Clipboard Sync Benchmark

Measure clipboard sync latency for text and image payloads, and verify echo-loop prevention:

```sh
cargo run -p hyperlink-bench -- clipboard --count 30 --images --image-size-kb 2048
```

> **Note**: simulated framing, hashing, and multiplexing — not a real OS clipboard on either side.

### 8. Run Phase 6 File Access Benchmark

Measure lazy random-access read latency (TTFB), write-back, and checksum integrity over a simulated large file:

```sh
cargo run -p hyperlink-bench -- file --file-size-mb 500 --seeks 20
```

> **Note**: simulated chunked read/write round-trip — not a real FUSE mount or Android storage backend.

### 9. Run Phase 7 Network Resilience & Multipath Benchmark

Simulate mid-session path failure and measure failover detection/switch latency:

```sh
cargo run -p hyperlink-bench -- resilience --scenarios 3 --max-failover-ms 1000
```

> **Note**: this is a deterministic protocol-state-machine simulation, not a real dual-network-interface failover test — see `CHANGELOG.md`'s Phase 7 entry for what that gap means in practice.

### 10. Run Phase 8 Proximity & Pre-Warmed Connect Benchmark

Compare simulated cold-connect vs. pre-warmed-connect setup latency:

```sh
cargo run -p hyperlink-bench -- proximity --iterations 5 --max-latency-ms 50
```

> **Note**: models an idealized version of pre-warming: see `docs/SYSTEM_DESIGN.md` Phase 8 and `CHANGELOG.md` for why the current beacon-over-an-already-open-connection design can't actually shorten real connection setup time yet.

### 11. Run Phase 9 Scoped App-State Handoff Benchmark

Measure handoff latency and state-restoration fidelity across browser tab, note draft, and media playback payloads:

```sh
cargo run -p hyperlink-bench -- handoff --iterations 10 --max-p95-ms 100
```

> **Note**: wire serialization and fidelity only — not a real Android intent or Linux `xdg-open` launch.

### 12. Run Phase 10 Ambient Context Agent Benchmark

Measure telemetry publish latency and verify per-category consent gating:

```sh
cargo run -p hyperlink-bench -- ambient --iterations 20 --max-p95-ms 10
```

> **Note**: exercises the event bus and consent-filter logic directly; not driven by real on-device telemetry.
