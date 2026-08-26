//! Android companion proximity beacon emitter and workflow state sync (Phase 8).
//!
//! Exposes helper functions for constructing and sending BLE/UWB proximity beacons,
//! tracking pre-warm state, and persisting workflow state.

use anyhow::Result;
use tracing::info;

use hyperlink_protocol::clock;
use hyperlink_protocol::proximity::{
    ProximityAck, ProximityBeacon, ProximityTechnology, WorkflowState,
};

/// Generates a ProximityBeacon from local ranging sensors.
pub fn create_proximity_beacon(
    cert_fingerprint: &str,
    technology_discriminant: u8,
    distance_cm: u16,
    rssi_dbm: i8,
    confidence_pct: u8,
    nonce: u64,
) -> Result<ProximityBeacon> {
    let tech = ProximityTechnology::try_from(technology_discriminant)?;

    // Parse cert fingerprint hex into 8-byte prefix
    let clean_hex = cert_fingerprint.replace([':', ' '], "");
    let mut fp_prefix = [0u8; 8];
    for (i, chunk) in clean_hex.as_bytes().chunks(2).take(8).enumerate() {
        if let Ok(s) = std::str::from_utf8(chunk) {
            if let Ok(b) = u8::from_str_radix(s, 16) {
                fp_prefix[i] = b;
            }
        }
    }

    Ok(ProximityBeacon {
        device_fingerprint_prefix: fp_prefix,
        technology: tech,
        distance_cm,
        rssi_dbm,
        confidence_pct,
        timestamp_us: clock::now_us(),
        nonce,
    })
}

/// Evaluates if a ProximityAck confirms successful pre-warming.
pub fn is_prewarm_confirmed(ack_bytes: &[u8]) -> bool {
    if let Ok(ack) = ProximityAck::decode(ack_bytes) {
        info!(
            accepted = ack.accepted,
            prewarm = ack.prewarm_initiated,
            "received proximity ack from host"
        );
        ack.accepted && ack.prewarm_initiated
    } else {
        false
    }
}

/// Creates a new WorkflowState snapshot representing current companion view.
pub fn capture_workflow_state(
    window_width: u32,
    window_height: u32,
    active_package_name: String,
    display_orientation: u8,
) -> WorkflowState {
    WorkflowState {
        window_width,
        window_height,
        window_x: 0,
        window_y: 0,
        is_fullscreen: false,
        active_package_name,
        display_orientation,
        saved_timestamp_us: clock::now_us(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_proximity_beacon_and_ack_verification() {
        let beacon = create_proximity_beacon(
            "AA:BB:CC:DD:EE:FF:11:22:33:44",
            2,  // UwbRanging
            40, // 40cm
            -35,
            99,
            98765,
        )
        .unwrap();

        assert_eq!(beacon.technology, ProximityTechnology::UwbRanging);
        assert_eq!(beacon.distance_cm, 40);
        assert_eq!(
            beacon.device_fingerprint_prefix,
            [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]
        );
        assert!(beacon.is_in_prewarm_range());

        let ack = ProximityAck {
            beacon_nonce: 98765,
            accepted: true,
            prewarm_initiated: true,
            ack_timestamp_us: clock::now_us(),
        };
        let mut ack_bytes = Vec::new();
        ack.encode(&mut ack_bytes).unwrap();
        assert!(is_prewarm_confirmed(&ack_bytes));
    }

    #[test]
    fn test_capture_workflow_state() {
        let wf = capture_workflow_state(1080, 2400, "com.sec.android.gallery3d".to_string(), 0);
        assert_eq!(wf.window_width, 1080);
        assert_eq!(wf.active_package_name, "com.sec.android.gallery3d");
        assert_eq!(wf.display_orientation, 0);
    }
}
