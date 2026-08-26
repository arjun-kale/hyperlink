//! Android companion state handoff builder and parser (Phase 9).
//!
//! Provides conversion between Android Java/Kotlin strings and strongly-typed
//! HyperLink handoff protocol wire representations.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use hyperlink_protocol::clock;
use hyperlink_protocol::handoff::{
    BrowserTabState, HandoffAck, HandoffPayload, MediaPlaybackState, NoteDraftState,
};

/// High-level representation of received handoff payload for JSON transmission to Android Kotlin layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AndroidHandoffEvent {
    pub session_id: u64,
    pub handoff_type: u8,
    pub source_origin: String,
    pub app_id: String,
    pub uri: String,
    pub title: String,
    pub state_json: String,
    pub timestamp_us: u64,
}

/// Builds a BrowserTab handoff payload.
pub fn create_browser_tab_handoff(
    session_id: u64,
    source_origin: &str,
    app_id: &str,
    url: &str,
    title: &str,
    scroll_y: u32,
) -> Result<HandoffPayload> {
    let tab = BrowserTabState {
        url: url.to_string(),
        title: title.to_string(),
        scroll_y,
        zoom_level: 1.0,
    };
    HandoffPayload::new_browser_tab(session_id, source_origin, app_id, &tab, clock::now_us())
}

/// Builds a NoteDraft handoff payload.
pub fn create_note_draft_handoff(
    session_id: u64,
    source_origin: &str,
    app_id: &str,
    uri: &str,
    title: &str,
    content: &str,
    cursor_offset: u32,
) -> Result<HandoffPayload> {
    let note = NoteDraftState {
        title: title.to_string(),
        content: content.to_string(),
        cursor_offset,
        is_pinned: false,
    };
    HandoffPayload::new_note_draft(
        session_id,
        source_origin,
        app_id,
        uri,
        &note,
        clock::now_us(),
    )
}

/// Builds a MediaPlayback handoff payload.
#[allow(clippy::too_many_arguments)]
pub fn create_media_playback_handoff(
    session_id: u64,
    source_origin: &str,
    app_id: &str,
    media_uri: &str,
    title: &str,
    position_ms: u64,
    duration_ms: u64,
    is_playing: bool,
) -> Result<HandoffPayload> {
    let media = MediaPlaybackState {
        media_uri: media_uri.to_string(),
        title: title.to_string(),
        position_ms,
        duration_ms,
        is_playing,
        volume: 1.0,
    };
    HandoffPayload::new_media_playback(session_id, source_origin, app_id, &media, clock::now_us())
}

/// Parses raw handoff wire bytes and serializes an AndroidHandoffEvent JSON string.
pub fn parse_handoff_to_json(payload_bytes: &[u8]) -> Result<String> {
    let payload = HandoffPayload::decode(payload_bytes)?;
    let event = AndroidHandoffEvent {
        session_id: payload.session_id,
        handoff_type: payload.handoff_type as u8,
        source_origin: payload.source_origin,
        app_id: payload.app_id,
        uri: payload.uri,
        title: payload.title,
        state_json: payload.state_json,
        timestamp_us: payload.timestamp_us,
    };
    Ok(serde_json::to_string(&event)?)
}

/// Creates a HandoffAck wire byte buffer.
pub fn create_handoff_ack(session_id: u64, accepted: bool, status_code: u8) -> Result<Vec<u8>> {
    let ack = HandoffAck {
        session_id,
        accepted,
        status_code,
        ack_timestamp_us: clock::now_us(),
    };
    let mut buf = Vec::with_capacity(HandoffAck::SIZE);
    ack.encode(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_parse_browser_handoff() {
        let payload = create_browser_tab_handoff(
            7001,
            "phone:GalaxyS24",
            "com.android.chrome",
            "https://rust-lang.org",
            "Rust Programming Language",
            200,
        )
        .unwrap();

        let mut wire_bytes = Vec::new();
        payload.encode(&mut wire_bytes).unwrap();

        let json = parse_handoff_to_json(&wire_bytes).unwrap();
        assert!(json.contains("\"session_id\":7001"));
        assert!(json.contains("\"uri\":\"https://rust-lang.org\""));
        assert!(json.contains("scroll_y"));
        assert!(json.contains("200"));
    }

    #[test]
    fn test_create_and_parse_note_handoff() {
        let payload = create_note_draft_handoff(
            7002,
            "host:LinuxPC",
            "org.gnome.TextEditor",
            "hyperlink-note://draft/7002",
            "Grocery List",
            "- Apples\n- Milk\n- Coffee",
            8,
        )
        .unwrap();

        let mut wire_bytes = Vec::new();
        payload.encode(&mut wire_bytes).unwrap();

        let json = parse_handoff_to_json(&wire_bytes).unwrap();
        assert!(json.contains("\"session_id\":7002"));
        assert!(json.contains("\"title\":\"Grocery List\""));
        assert!(json.contains("Coffee"));
    }

    #[test]
    fn test_create_handoff_ack() {
        let ack_bytes = create_handoff_ack(7001, true, 0).unwrap();
        let ack = HandoffAck::decode(&ack_bytes).unwrap();
        assert_eq!(ack.session_id, 7001);
        assert!(ack.accepted);
        assert_eq!(ack.status_code, 0);
    }
}
