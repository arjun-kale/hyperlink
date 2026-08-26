//! Scoped App-State Handoff protocol wire types and schema validation (Phase 9).
//!
//! Provides a structured, documented contract for transferring mid-task application
//! state (e.g. browser tabs, note drafts, media playback positions, and custom deep-links)
//! between Linux and Android devices.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Maximum allowable string length limits to prevent resource exhaustion attacks.
pub const MAX_APP_ID_LEN: usize = 256;
pub const MAX_URI_LEN: usize = 4096;
pub const MAX_TITLE_LEN: usize = 1024;
pub const MAX_STATE_JSON_LEN: usize = 65536; // 64 KB state payload ceiling

/// Supported categories of scoped app-state handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum HandoffType {
    /// Active web browser tab (URL, scroll position, page title).
    BrowserTab = 1,
    /// Text note or draft document (title, markdown/text body, cursor position).
    NoteDraft = 2,
    /// Media playback session (audio/video stream URI, playback position ms, playback state).
    MediaPlayback = 3,
    /// Custom third-party app deep-link URI with arbitrary structured JSON state.
    CustomUri = 4,
}

impl TryFrom<u8> for HandoffType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::BrowserTab),
            2 => Ok(Self::NoteDraft),
            3 => Ok(Self::MediaPlayback),
            4 => Ok(Self::CustomUri),
            other => Err(anyhow!("unsupported HandoffType discriminant: {}", other)),
        }
    }
}

/// Structured state payload for BrowserTab handoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BrowserTabState {
    pub url: String,
    pub title: String,
    pub scroll_y: u32,
    pub zoom_level: f32,
}

/// Structured state payload for NoteDraft handoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NoteDraftState {
    pub title: String,
    pub content: String,
    pub cursor_offset: u32,
    pub is_pinned: bool,
}

/// Structured state payload for MediaPlayback handoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaPlaybackState {
    pub media_uri: String,
    pub title: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub is_playing: bool,
    pub volume: f32,
}

/// Main wire payload for transferring application state across peer devices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffPayload {
    /// Unique correlation ID for the handoff session.
    pub session_id: u64,
    /// Origin device identifier (e.g. `phone:GalaxyS24` or `host:LinuxDesktop`).
    pub source_origin: String,
    /// Category of handoff.
    pub handoff_type: HandoffType,
    /// Target application or package identifier (e.g. `org.mozilla.firefox`).
    pub app_id: String,
    /// Primary URI or deep-link to launch.
    pub uri: String,
    /// Human-readable title of the handed-off task/document.
    pub title: String,
    /// JSON-encoded state metadata.
    pub state_json: String,
    /// Microsecond UTC timestamp when handoff was triggered.
    pub timestamp_us: u64,
}

impl HandoffPayload {
    /// Creates a new BrowserTab handoff payload.
    pub fn new_browser_tab(
        session_id: u64,
        source_origin: impl Into<String>,
        app_id: impl Into<String>,
        tab_state: &BrowserTabState,
        timestamp_us: u64,
    ) -> Result<Self> {
        let state_json = serde_json::to_string(tab_state)?;
        Ok(Self {
            session_id,
            source_origin: source_origin.into(),
            handoff_type: HandoffType::BrowserTab,
            app_id: app_id.into(),
            uri: tab_state.url.clone(),
            title: tab_state.title.clone(),
            state_json,
            timestamp_us,
        })
    }

    /// Creates a new NoteDraft handoff payload.
    pub fn new_note_draft(
        session_id: u64,
        source_origin: impl Into<String>,
        app_id: impl Into<String>,
        uri: impl Into<String>,
        note_state: &NoteDraftState,
        timestamp_us: u64,
    ) -> Result<Self> {
        let state_json = serde_json::to_string(note_state)?;
        Ok(Self {
            session_id,
            source_origin: source_origin.into(),
            handoff_type: HandoffType::NoteDraft,
            app_id: app_id.into(),
            uri: uri.into(),
            title: note_state.title.clone(),
            state_json,
            timestamp_us,
        })
    }

    /// Creates a new MediaPlayback handoff payload.
    pub fn new_media_playback(
        session_id: u64,
        source_origin: impl Into<String>,
        app_id: impl Into<String>,
        media_state: &MediaPlaybackState,
        timestamp_us: u64,
    ) -> Result<Self> {
        let state_json = serde_json::to_string(media_state)?;
        Ok(Self {
            session_id,
            source_origin: source_origin.into(),
            handoff_type: HandoffType::MediaPlayback,
            app_id: app_id.into(),
            uri: media_state.media_uri.clone(),
            title: media_state.title.clone(),
            state_json,
            timestamp_us,
        })
    }

    /// Validates field constraints against protocol limits.
    pub fn validate(&self) -> Result<()> {
        if self.source_origin.len() > MAX_APP_ID_LEN {
            return Err(anyhow!("source_origin exceeds maximum length"));
        }
        if self.app_id.len() > MAX_APP_ID_LEN {
            return Err(anyhow!("app_id exceeds maximum length"));
        }
        if self.uri.len() > MAX_URI_LEN {
            return Err(anyhow!("uri exceeds maximum length"));
        }
        if self.title.len() > MAX_TITLE_LEN {
            return Err(anyhow!("title exceeds maximum length"));
        }
        if self.state_json.len() > MAX_STATE_JSON_LEN {
            return Err(anyhow!("state_json exceeds maximum length"));
        }
        Ok(())
    }

    /// Encodes the payload into a compact binary wire format.
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        self.validate()?;

        buf.extend_from_slice(&self.session_id.to_be_bytes());

        let origin_bytes = self.source_origin.as_bytes();
        buf.extend_from_slice(&(origin_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(origin_bytes);

        buf.push(self.handoff_type as u8);

        let app_bytes = self.app_id.as_bytes();
        buf.extend_from_slice(&(app_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(app_bytes);

        let uri_bytes = self.uri.as_bytes();
        buf.extend_from_slice(&(uri_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(uri_bytes);

        let title_bytes = self.title.as_bytes();
        buf.extend_from_slice(&(title_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(title_bytes);

        let state_bytes = self.state_json.as_bytes();
        buf.extend_from_slice(&(state_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(state_bytes);

        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());
        Ok(())
    }

    /// Decodes the payload from a binary wire format.
    pub fn decode(mut buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 + 2 + 1 + 2 + 2 + 2 + 4 + 8 {
            return Err(anyhow!("payload too short for HandoffPayload"));
        }

        let session_id = u64::from_be_bytes(buf[..8].try_into()?);
        buf = &buf[8..];

        let origin_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < origin_len {
            return Err(anyhow!("unexpected EOF reading source_origin"));
        }
        let source_origin = std::str::from_utf8(&buf[..origin_len])?.to_string();
        buf = &buf[origin_len..];

        let handoff_type = HandoffType::try_from(buf[0])?;
        buf = &buf[1..];

        let app_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < app_len {
            return Err(anyhow!("unexpected EOF reading app_id"));
        }
        let app_id = std::str::from_utf8(&buf[..app_len])?.to_string();
        buf = &buf[app_len..];

        let uri_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < uri_len {
            return Err(anyhow!("unexpected EOF reading uri"));
        }
        let uri = std::str::from_utf8(&buf[..uri_len])?.to_string();
        buf = &buf[uri_len..];

        let title_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < title_len {
            return Err(anyhow!("unexpected EOF reading title"));
        }
        let title = std::str::from_utf8(&buf[..title_len])?.to_string();
        buf = &buf[title_len..];

        let state_len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
        buf = &buf[4..];
        if buf.len() < state_len {
            return Err(anyhow!("unexpected EOF reading state_json"));
        }
        let state_json = std::str::from_utf8(&buf[..state_len])?.to_string();
        buf = &buf[state_len..];

        if buf.len() < 8 {
            return Err(anyhow!("unexpected EOF reading timestamp_us"));
        }
        let timestamp_us = u64::from_be_bytes(buf[..8].try_into()?);

        let payload = Self {
            session_id,
            source_origin,
            handoff_type,
            app_id,
            uri,
            title,
            state_json,
            timestamp_us,
        };
        payload.validate()?;
        Ok(payload)
    }
}

/// Acknowledgement confirming receipt and dispatch of state handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffAck {
    /// Matching session ID from the handoff request.
    pub session_id: u64,
    /// Whether the peer accepted and dispatched the handoff.
    pub accepted: bool,
    /// Result status code:
    /// 0 = Success / Launched
    /// 1 = Unsupported Handoff Type
    /// 2 = Missing / Uninstalled Target Handler
    /// 3 = Policy Rejected / User Denied
    pub status_code: u8,
    /// UTC microsecond timestamp of acknowledgement.
    pub ack_timestamp_us: u64,
}

impl HandoffAck {
    pub const SIZE: usize = 8 + 1 + 1 + 8; // 18 bytes

    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(&self.session_id.to_be_bytes());
        buf.push(if self.accepted { 1 } else { 0 });
        buf.push(self.status_code);
        buf.extend_from_slice(&self.ack_timestamp_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < Self::SIZE {
            return Err(anyhow!("buffer too short for HandoffAck"));
        }
        let session_id = u64::from_be_bytes(buf[..8].try_into()?);
        let accepted = buf[8] != 0;
        let status_code = buf[9];
        let ack_timestamp_us = u64::from_be_bytes(buf[10..18].try_into()?);
        Ok(Self {
            session_id,
            accepted,
            status_code,
            ack_timestamp_us,
        })
    }
}

/// Capability discovery for negotiated handoff support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HandoffCapability {
    pub supported_types: Vec<u8>,
    pub supported_schemes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_tab_handoff_round_trip() {
        let tab = BrowserTabState {
            url: "https://news.ycombinator.com/item?id=40000000".to_string(),
            title: "Hacker News".to_string(),
            scroll_y: 1250,
            zoom_level: 1.0,
        };

        let payload = HandoffPayload::new_browser_tab(
            1001,
            "phone:GalaxyS24",
            "org.mozilla.firefox",
            &tab,
            123456789,
        )
        .unwrap();

        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();

        let decoded = HandoffPayload::decode(&buf).unwrap();
        assert_eq!(decoded.session_id, 1001);
        assert_eq!(decoded.handoff_type, HandoffType::BrowserTab);
        assert_eq!(decoded.uri, tab.url);
        assert_eq!(decoded.title, tab.title);

        let decoded_tab: BrowserTabState = serde_json::from_str(&decoded.state_json).unwrap();
        assert_eq!(decoded_tab, tab);
    }

    #[test]
    fn test_note_draft_handoff_round_trip() {
        let note = NoteDraftState {
            title: "Meeting Notes 2026".to_string(),
            content: "Discuss HyperLink Phase 9 architecture and benchmarks.".to_string(),
            cursor_offset: 45,
            is_pinned: true,
        };

        let payload = HandoffPayload::new_note_draft(
            1002,
            "host:ArchLinux",
            "com.google.android.keep",
            "hyperlink-note://draft/1002",
            &note,
            987654321,
        )
        .unwrap();

        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();

        let decoded = HandoffPayload::decode(&buf).unwrap();
        assert_eq!(decoded.session_id, 1002);
        assert_eq!(decoded.handoff_type, HandoffType::NoteDraft);
        assert_eq!(decoded.title, "Meeting Notes 2026");

        let decoded_note: NoteDraftState = serde_json::from_str(&decoded.state_json).unwrap();
        assert_eq!(decoded_note, note);
    }

    #[test]
    fn test_media_playback_handoff_round_trip() {
        let media = MediaPlaybackState {
            media_uri: "https://streaming.example.com/podcast/ep42.mp3".to_string(),
            title: "Episode 42: The Future of Compute Surfaces".to_string(),
            position_ms: 124500,
            duration_ms: 3600000,
            is_playing: true,
            volume: 0.8,
        };

        let payload = HandoffPayload::new_media_playback(
            1003,
            "phone:GalaxyS24",
            "org.videolan.vlc",
            &media,
            555555,
        )
        .unwrap();

        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();

        let decoded = HandoffPayload::decode(&buf).unwrap();
        assert_eq!(decoded.session_id, 1003);
        assert_eq!(decoded.handoff_type, HandoffType::MediaPlayback);

        let decoded_media: MediaPlaybackState = serde_json::from_str(&decoded.state_json).unwrap();
        assert_eq!(decoded_media, media);
    }

    #[test]
    fn test_handoff_ack_round_trip() {
        let ack = HandoffAck {
            session_id: 1001,
            accepted: true,
            status_code: 0,
            ack_timestamp_us: 123456789,
        };

        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), HandoffAck::SIZE);

        let decoded = HandoffAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn test_oversize_state_payload_rejected() {
        let oversize_json = "x".repeat(MAX_STATE_JSON_LEN + 10);
        let payload = HandoffPayload {
            session_id: 999,
            source_origin: "phone".to_string(),
            handoff_type: HandoffType::CustomUri,
            app_id: "com.example.app".to_string(),
            uri: "https://example.com".to_string(),
            title: "Test".to_string(),
            state_json: oversize_json,
            timestamp_us: 1000,
        };

        let mut buf = Vec::new();
        assert!(payload.encode(&mut buf).is_err());
    }
}
