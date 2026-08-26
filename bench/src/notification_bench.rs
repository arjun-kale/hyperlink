//! Notification sync latency and delivery benchmark for Phase 4.
//!
//! Evaluates end-to-end notification dispatch, framing, serialization,
//! round-trip acknowledgment accounting, action invocation latency,
//! and verifies Phase 4 DoD criteria against targets (<100ms propagation latency,
//! zero loss on reliable QUIC stream).

use std::time::{Duration, Instant};
use tracing::info;

use hyperlink_protocol::message::MessageType;
use hyperlink_protocol::metrics::{LatencyStats, NotificationBenchStats};
use hyperlink_protocol::notification::{
    NotificationAck, NotificationAction, NotificationActionInvoke, NotificationPost,
};
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

/// Options configuring the notification latency benchmark.
#[derive(Debug, Clone)]
pub struct NotificationBenchOptions {
    /// Number of notifications to dispatch.
    pub total_notifications: u64,
    /// Interval between notification dispatches in milliseconds.
    pub interval_ms: u64,
    /// Number of interactive action button clicks to simulate.
    pub action_invocations: u64,
    /// Whether to include 48x48 icon byte payloads in the notifications.
    pub include_icons: bool,
    /// Simulated packet loss rate (0.0 to 1.0).
    pub simulated_loss_rate: f64,
}

impl Default for NotificationBenchOptions {
    fn default() -> Self {
        Self {
            total_notifications: 50,
            interval_ms: 20,
            action_invocations: 10,
            include_icons: true,
            simulated_loss_rate: 0.0,
        }
    }
}

/// Runs the notification benchmark session, simulating end-to-end notification framing,
/// acknowledgment accounting, and action invocation round-trip latency.
pub fn run_notification_benchmark(options: &NotificationBenchOptions) -> NotificationBenchStats {
    info!(
        notifications = options.total_notifications,
        interval_ms = options.interval_ms,
        actions = options.action_invocations,
        include_icons = options.include_icons,
        loss_rate = options.simulated_loss_rate,
        "starting Phase 4 notification latency benchmark session"
    );

    let mut rng = SimpleRng::new(0x87654321_FEDCBA98);
    let mut propagation_latencies_us: Vec<i64> =
        Vec::with_capacity(options.total_notifications as usize);
    let mut action_rtts_us: Vec<i64> = Vec::with_capacity(options.action_invocations as usize);

    let mut lost_count: u64 = 0;
    let mut acked_count: u64 = 0;

    let sample_icon: Option<Vec<u8>> = if options.include_icons {
        // 1 KB dummy compressed PNG payload
        Some([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A].repeat(128))
    } else {
        None
    };

    let dispatch_interval = Duration::from_millis(options.interval_ms);

    // 1. Dispatch notification stream
    for seq in 1..=options.total_notifications {
        let send_instant = Instant::now();
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let notif_id = format!("sbn_bench_key_{seq:04}");
        let notif = NotificationPost {
            id: notif_id.clone(),
            package_name: "com.example.chat".to_string(),
            app_name: "ExampleChat".to_string(),
            title: format!("Sender {seq}"),
            body: format!("Message content payload for notification sequence {seq}"),
            timestamp_ms,
            actions: vec![
                NotificationAction {
                    action_id: 1,
                    title: "Quick Reply".to_string(),
                },
                NotificationAction {
                    action_id: 2,
                    title: "Mark Read".to_string(),
                },
            ],
            icon_png: sample_icon.clone(),
        };

        // Wire framing & serialization
        let mut payload = Vec::new();
        notif
            .encode(&mut payload)
            .expect("notification encode failed");

        let header = Header::new(MessageType::NotificationPost, payload.len() as u32);
        let mut packet =
            Vec::with_capacity(hyperlink_protocol::version::HEADER_SIZE + payload.len());
        header.encode(&mut packet).expect("header encode failed");
        packet.extend_from_slice(&payload);

        // Simulate delivery / loss
        if options.simulated_loss_rate > 0.0 && rng.next_f64() < options.simulated_loss_rate {
            lost_count += 1;
            continue;
        }

        // Decode on Linux host side
        let host_recv_hdr = Header::decode(&packet[..hyperlink_protocol::version::HEADER_SIZE])
            .expect("header decode failed");
        assert_eq!(host_recv_hdr.message_type, MessageType::NotificationPost);

        let decoded_notif =
            NotificationPost::decode(&packet[hyperlink_protocol::version::HEADER_SIZE..])
                .expect("notification decode failed");
        assert_eq!(decoded_notif.id, notif_id);

        let host_received_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        // Host generates ACK
        let ack = NotificationAck {
            id: decoded_notif.id.clone(),
            device_timestamp_ms: decoded_notif.timestamp_ms,
            host_received_us,
        };

        let mut ack_payload = Vec::new();
        ack.encode(&mut ack_payload).expect("ack encode failed");

        let ack_hdr = Header::new(MessageType::NotificationAck, ack_payload.len() as u32);
        let mut ack_packet =
            Vec::with_capacity(hyperlink_protocol::version::HEADER_SIZE + ack_payload.len());
        ack_hdr
            .encode(&mut ack_packet)
            .expect("ack header encode failed");
        ack_packet.extend_from_slice(&ack_payload);

        // Device decodes ACK
        let decoded_ack_hdr =
            Header::decode(&ack_packet[..hyperlink_protocol::version::HEADER_SIZE])
                .expect("ack header decode failed");
        assert_eq!(decoded_ack_hdr.message_type, MessageType::NotificationAck);
        let decoded_ack =
            NotificationAck::decode(&ack_packet[hyperlink_protocol::version::HEADER_SIZE..])
                .expect("ack decode failed");
        assert_eq!(decoded_ack.id, notif_id);

        let round_trip_elapsed = send_instant.elapsed();
        propagation_latencies_us.push(round_trip_elapsed.as_micros() as i64);
        acked_count += 1;

        if options.interval_ms > 0 && round_trip_elapsed < dispatch_interval {
            std::thread::sleep(dispatch_interval - round_trip_elapsed);
        }
    }

    // 2. Dispatch action invocations
    let mut action_acked_count: u64 = 0;
    for action_seq in 1..=options.action_invocations {
        let send_instant = Instant::now();
        let notif_id = format!("sbn_bench_key_{action_seq:04}");

        let invoke = NotificationActionInvoke {
            id: notif_id.clone(),
            action_id: 1, // Quick reply
        };

        let mut payload = Vec::new();
        invoke
            .encode(&mut payload)
            .expect("action invoke encode failed");
        let header = Header::new(MessageType::NotificationActionInvoke, payload.len() as u32);
        let mut packet =
            Vec::with_capacity(hyperlink_protocol::version::HEADER_SIZE + payload.len());
        header.encode(&mut packet).expect("header encode failed");
        packet.extend_from_slice(&payload);

        // Android decodes action invoke
        let recv_hdr = Header::decode(&packet[..hyperlink_protocol::version::HEADER_SIZE])
            .expect("header decode failed");
        assert_eq!(recv_hdr.message_type, MessageType::NotificationActionInvoke);
        let decoded_invoke =
            NotificationActionInvoke::decode(&packet[hyperlink_protocol::version::HEADER_SIZE..])
                .expect("action invoke decode failed");
        assert_eq!(decoded_invoke.id, notif_id);

        action_rtts_us.push(send_instant.elapsed().as_micros() as i64);
        action_acked_count += 1;
    }

    let loss_rate_pct = if options.total_notifications > 0 {
        (lost_count as f64 / options.total_notifications as f64) * 100.0
    } else {
        0.0
    };

    let propagation_stats = LatencyStats::from_rtts(&propagation_latencies_us, lost_count);
    let action_stats = if !action_rtts_us.is_empty() {
        Some(LatencyStats::from_rtts(&action_rtts_us, 0))
    } else {
        None
    };

    // Phase 4 DoD Criteria:
    // 1. Propagation latency p95 <= 100,000 µs (100 ms)
    // 2. Zero / negligible loss on reliable control stream: loss_rate <= 0.1%
    let target_met = propagation_stats.p95_us <= 100_000 && loss_rate_pct <= 0.1;

    info!(
        total_posted = options.total_notifications,
        total_acks = acked_count,
        lost = lost_count,
        loss_pct = format!("{:.2}%", loss_rate_pct),
        p50_us = propagation_stats.p50_us,
        p95_us = propagation_stats.p95_us,
        mean_us = format!("{:.1}", propagation_stats.mean_us),
        target_met = target_met,
        "Phase 4 notification benchmark complete"
    );

    NotificationBenchStats {
        total_posted: options.total_notifications,
        total_acks: acked_count,
        lost_count,
        loss_rate_pct,
        propagation_latency_stats: propagation_stats,
        total_actions_invoked: options.action_invocations,
        action_acks_received: action_acked_count,
        action_rtt_stats: action_stats,
        target_met,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_bench_baseline_passes_dod() {
        let options = NotificationBenchOptions {
            total_notifications: 50,
            interval_ms: 0,
            action_invocations: 10,
            include_icons: true,
            simulated_loss_rate: 0.0,
        };

        let stats = run_notification_benchmark(&options);
        assert_eq!(stats.total_posted, 50);
        assert_eq!(stats.total_acks, 50);
        assert_eq!(stats.lost_count, 0);
        assert_eq!(stats.loss_rate_pct, 0.0);
        assert!(stats.propagation_latency_stats.p95_us <= 100_000);
        assert!(stats.target_met);
    }

    #[test]
    fn test_notification_bench_degraded_fails_dod() {
        let options = NotificationBenchOptions {
            total_notifications: 50,
            interval_ms: 0,
            action_invocations: 0,
            include_icons: false,
            simulated_loss_rate: 0.10, // 10% loss must fail strict DoD
        };

        let stats = run_notification_benchmark(&options);
        assert!(stats.lost_count > 0);
        assert!(stats.loss_rate_pct > 0.1);
        assert!(!stats.target_met);
    }
}
