//! Android companion ambient context event serialization and bridge (Phase 10).
//!
//! Exposes helper functions for constructing and validating ambient telemetry events
//! sent from Android to the Linux host.

use anyhow::Result;

use hyperlink_protocol::ambient::{AmbientEvent, AmbientEventCategory};
use hyperlink_protocol::clock;

/// Constructs an AmbientEvent from raw Android telemetry data.
pub fn create_ambient_event(
    event_id: u64,
    category_id: u8,
    source: &str,
    summary: &str,
    metadata_json: &str,
) -> Result<AmbientEvent> {
    let category = AmbientEventCategory::try_from(category_id)?;
    AmbientEvent::new(
        event_id,
        category,
        source,
        clock::now_us(),
        summary,
        metadata_json,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_ambient_event() {
        let event = create_ambient_event(
            9001,
            2, // ScreenState
            "phone:GalaxyS24",
            "Screen Unlocked (Orientation: Portrait)",
            r#"{"screen_on":true,"locked":false,"orientation":1}"#,
        )
        .unwrap();

        assert_eq!(event.event_id, 9001);
        assert_eq!(event.category, AmbientEventCategory::ScreenState);
        assert!(event.summary.contains("Screen Unlocked"));
    }
}
