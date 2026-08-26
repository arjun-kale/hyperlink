//! Input stream latency benchmark for Phase 3.
//!
//! Emulates dispatching high-frequency pointer and keyboard events across the
//! reliable input channel, recording round-trip delivery acknowledgements (`InputAck`),
//! calculating round-trip latency percentiles, and verifying DoD criteria
//! against `SYSTEM_DESIGN.md` targets (<15ms USB, <30ms 5GHz WiFi).

use std::time::{Duration, Instant};
use tracing::info;

use hyperlink_protocol::input::{
    InputAck, KeyAction, KeyEvent, PointerAction, PointerButton, PointerEvent,
};
use hyperlink_protocol::message::MessageType;
use hyperlink_protocol::metrics::{InputBenchStats, LatencyStats};
use hyperlink_protocol::version::Header;

/// Simple deterministic RNG for benchmark packet loss simulation without external crates.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.state >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

/// Options configuring the input latency benchmark.
#[derive(Debug, Clone)]
pub struct InputBenchOptions {
    /// Number of input events to dispatch.
    pub total_events: u64,
    /// Interval between events in milliseconds (e.g. 8ms = ~125 Hz input rate).
    pub interval_ms: u64,
    /// Simulated packet loss rate (0.0 to 1.0).
    pub simulated_loss_rate: f64,
}

impl Default for InputBenchOptions {
    fn default() -> Self {
        Self {
            total_events: 200,
            interval_ms: 8,
            simulated_loss_rate: 0.0,
        }
    }
}

/// Runs the input benchmark session, simulating end-to-end event framing, serialization,
/// and acknowledgment round-trip accounting.
pub fn run_input_benchmark(options: &InputBenchOptions) -> InputBenchStats {
    info!(
        events = options.total_events,
        interval_ms = options.interval_ms,
        loss_rate = options.simulated_loss_rate,
        "starting Phase 3 input latency benchmark session"
    );

    let mut rng = SimpleRng::new(0x12345678_ABCDEF01);
    let mut completed_rtts_us: Vec<i64> = Vec::with_capacity(options.total_events as usize);
    let mut lost_count: u64 = 0;
    let mut acked_count: u64 = 0;

    let event_interval = Duration::from_millis(options.interval_ms);

    for seq in 1..=options.total_events {
        let send_instant = Instant::now();
        let timestamp_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        // Alternate between PointerEvent and KeyEvent
        let mut payload = Vec::new();
        let (msg_type, enc_res) = if seq % 2 == 1 {
            let p = PointerEvent {
                action: PointerAction::Move,
                button: PointerButton::Primary,
                x_norm: PointerEvent::float_to_norm(
                    (seq as f64 / options.total_events as f64).fract(),
                ),
                y_norm: PointerEvent::float_to_norm(
                    ((seq * 2) as f64 / options.total_events as f64).fract(),
                ),
                pressure: 128,
                timestamp_us,
            };
            (MessageType::PointerEvent, p.encode(&mut payload))
        } else {
            let k = KeyEvent {
                action: KeyAction::Down,
                keycode: (65 + (seq % 26)) as u32,
                modifiers: 0,
                timestamp_us,
            };
            (MessageType::KeyEvent, k.encode(&mut payload))
        };
        assert!(enc_res.is_ok(), "failed to encode input event");

        let header = Header::new(msg_type, payload.len() as u32);
        let mut wire_packet = Vec::with_capacity(10 + payload.len());
        header.encode(&mut wire_packet).unwrap();
        wire_packet.extend_from_slice(&payload);

        // Simulated loss gate
        let is_lost =
            options.simulated_loss_rate > 0.0 && rng.next_f64() < options.simulated_loss_rate;

        if is_lost {
            lost_count += 1;
        } else {
            // Receiver decodes header and payload
            let decoded_header = Header::decode(&wire_packet[..10]).unwrap();
            match decoded_header.message_type {
                MessageType::PointerEvent => {
                    let _ = PointerEvent::decode(&wire_packet[10..]).unwrap();
                }
                MessageType::KeyEvent => {
                    let _ = KeyEvent::decode(&wire_packet[10..]).unwrap();
                }
                _ => panic!("unexpected message type in input benchmark"),
            }

            // Receiver generates InputAck
            let ack = InputAck {
                seq: seq as u32,
                timestamp_us,
            };
            let mut ack_payload = Vec::new();
            ack.encode(&mut ack_payload).unwrap();
            let ack_header = Header::new(MessageType::InputAck, ack_payload.len() as u32);
            let mut ack_packet = Vec::with_capacity(10 + ack_payload.len());
            ack_header.encode(&mut ack_packet).unwrap();
            ack_packet.extend_from_slice(&ack_payload);

            // Sender decodes InputAck and calculates RTT
            let _ = InputAck::decode(&ack_packet[10..]).unwrap();
            let rtt_us = send_instant.elapsed().as_micros() as i64;
            completed_rtts_us.push(rtt_us.max(1));
            acked_count += 1;
        }

        // Pacing
        let elapsed = send_instant.elapsed();
        if elapsed < event_interval {
            std::thread::sleep(event_interval - elapsed);
        }
    }

    let loss_rate_pct = if options.total_events > 0 {
        (lost_count as f64 / options.total_events as f64) * 100.0
    } else {
        0.0
    };

    let rtt_stats = if !completed_rtts_us.is_empty() {
        LatencyStats::from_rtts(&completed_rtts_us, lost_count)
    } else {
        LatencyStats {
            count: 0,
            min_us: 0,
            max_us: 0,
            mean_us: 0.0,
            p50_us: 0,
            p95_us: 0,
            p99_us: 0,
            stddev_us: 0.0,
            lost: lost_count,
        }
    };

    // Phase 3 DoD target verification:
    // Input RTT target: <15ms USB (15,000µs), <30ms WiFi (30,000µs)
    // Loss threshold: <= 1.0%
    let target_met =
        !completed_rtts_us.is_empty() && (rtt_stats.p95_us <= 30_000) && (loss_rate_pct <= 1.0);

    InputBenchStats {
        total_events: options.total_events,
        total_acks: acked_count,
        lost_events: lost_count,
        loss_rate_pct,
        rtt_stats,
        target_met,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_input_benchmark() {
        let options = InputBenchOptions {
            total_events: 50,
            interval_ms: 1,
            simulated_loss_rate: 0.0,
        };
        let stats = run_input_benchmark(&options);
        assert_eq!(stats.total_events, 50);
        assert_eq!(stats.total_acks, 50);
        assert_eq!(stats.lost_events, 0);
        assert!(stats.target_met);
        assert!(stats.rtt_stats.p95_us < 30_000);
    }

    #[test]
    fn test_lossy_input_benchmark() {
        let options = InputBenchOptions {
            total_events: 50,
            interval_ms: 1,
            simulated_loss_rate: 0.10, // 10% simulated loss
        };
        let stats = run_input_benchmark(&options);
        assert_eq!(stats.total_events, 50);
        assert!(stats.lost_events > 0);
        // Breaches loss threshold, so target_met is correctly false
        assert!(!stats.target_met);
    }
}
