//! Ambient Context Agent wire types, category schemas, and consent gating (Phase 10).
//!
//! Exposes a privacy-gated internal event bus (notifications, screen-state metadata,
//! foreground app activity, device status, clipboard, media playback) that desktop AI
//! agents can query or subscribe to with strict per-category consent and raw-video sandboxing.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Maximum allowable length for an ambient event source origin string.
pub const MAX_SOURCE_LEN: usize = 256;
/// Maximum allowable length for an ambient event human-readable summary.
pub const MAX_SUMMARY_LEN: usize = 1024;
/// Maximum allowable length for an ambient event metadata JSON payload (16 KB).
pub const MAX_METADATA_JSON_LEN: usize = 16384;
/// Maximum allowable length for an agent query string (4 KB).
pub const MAX_QUERY_LEN: usize = 4096;
/// Maximum allowable length for an agent response answer string (64 KB).
pub const MAX_ANSWER_LEN: usize = 65536;

/// Ambient event category classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AmbientEventCategory {
    /// Notification events (app name, title, body snippet, action state).
    Notifications = 1,
    /// Screen state metadata (screen on/off, lock status, display orientation, resolution).
    ScreenState = 2,
    /// Foreground app metadata (active package, activity name, category).
    ForegroundApp = 3,
    /// Device status telemetry (battery level, charging status, network type, thermal level).
    DeviceStatus = 4,
    /// Clipboard activity preview (content type, preview snippet, origin).
    Clipboard = 5,
    /// Media playback status (track title, artist, playback state, position).
    MediaState = 6,
}

impl TryFrom<u8> for AmbientEventCategory {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Notifications),
            2 => Ok(Self::ScreenState),
            3 => Ok(Self::ForegroundApp),
            4 => Ok(Self::DeviceStatus),
            5 => Ok(Self::Clipboard),
            6 => Ok(Self::MediaState),
            other => Err(anyhow!("unsupported ambient event category: {}", other)),
        }
    }
}

/// A structured ambient context event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbientEvent {
    /// Monotonically increasing unique event ID.
    pub event_id: u64,
    /// Event category classification.
    pub category: AmbientEventCategory,
    /// Source origin identifier (e.g. "phone:GalaxyS24" or "host:LinuxDesktop").
    pub source: String,
    /// UTC timestamp of occurrence in microseconds.
    pub timestamp_us: u64,
    /// Human-readable event summary for quick agent indexing.
    pub summary: String,
    /// Structured metadata JSON string conforming to the category schema.
    pub metadata_json: String,
}

impl AmbientEvent {
    /// Creates a new AmbientEvent after validating string bounds.
    pub fn new(
        event_id: u64,
        category: AmbientEventCategory,
        source: impl Into<String>,
        timestamp_us: u64,
        summary: impl Into<String>,
        metadata_json: impl Into<String>,
    ) -> Result<Self> {
        let event = Self {
            event_id,
            category,
            source: source.into(),
            timestamp_us,
            summary: summary.into(),
            metadata_json: metadata_json.into(),
        };
        event.validate()?;
        Ok(event)
    }

    /// Validates payload length constraints.
    pub fn validate(&self) -> Result<()> {
        if self.source.len() > MAX_SOURCE_LEN {
            return Err(anyhow!(
                "event source exceeds max length ({} > {})",
                self.source.len(),
                MAX_SOURCE_LEN
            ));
        }
        if self.summary.len() > MAX_SUMMARY_LEN {
            return Err(anyhow!(
                "event summary exceeds max length ({} > {})",
                self.summary.len(),
                MAX_SUMMARY_LEN
            ));
        }
        if self.metadata_json.len() > MAX_METADATA_JSON_LEN {
            return Err(anyhow!(
                "event metadata JSON exceeds max length ({} > {})",
                self.metadata_json.len(),
                MAX_METADATA_JSON_LEN
            ));
        }
        Ok(())
    }

    /// Binary wire encoding.
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        self.validate()?;
        buf.extend_from_slice(&self.event_id.to_be_bytes());
        buf.push(self.category as u8);

        let source_bytes = self.source.as_bytes();
        buf.extend_from_slice(&(source_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(source_bytes);

        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());

        let summary_bytes = self.summary.as_bytes();
        buf.extend_from_slice(&(summary_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(summary_bytes);

        let meta_bytes = self.metadata_json.as_bytes();
        buf.extend_from_slice(&(meta_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(meta_bytes);
        Ok(())
    }

    /// Binary wire decoding.
    pub fn decode(mut buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 + 1 + 2 + 8 + 2 + 4 {
            return Err(anyhow!(
                "ambient event payload too short: {} bytes",
                buf.len()
            ));
        }

        let event_id = u64::from_be_bytes(buf[..8].try_into()?);
        buf = &buf[8..];

        let category = AmbientEventCategory::try_from(buf[0])?;
        buf = &buf[1..];

        let source_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < source_len {
            return Err(anyhow!("unexpected EOF reading event source"));
        }
        let source = std::str::from_utf8(&buf[..source_len])?.to_string();
        buf = &buf[source_len..];

        if buf.len() < 8 + 2 + 4 {
            return Err(anyhow!("ambient event truncated before timestamp"));
        }
        let timestamp_us = u64::from_be_bytes(buf[..8].try_into()?);
        buf = &buf[8..];

        let summary_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < summary_len {
            return Err(anyhow!("unexpected EOF reading event summary"));
        }
        let summary = std::str::from_utf8(&buf[..summary_len])?.to_string();
        buf = &buf[summary_len..];

        if buf.len() < 4 {
            return Err(anyhow!("ambient event truncated before metadata_json"));
        }
        let meta_len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
        buf = &buf[4..];
        if buf.len() < meta_len {
            return Err(anyhow!("unexpected EOF reading event metadata_json"));
        }
        let metadata_json = std::str::from_utf8(&buf[..meta_len])?.to_string();

        let event = Self {
            event_id,
            category,
            source,
            timestamp_us,
            summary,
            metadata_json,
        };
        event.validate()?;
        Ok(event)
    }
}

/// Granular agent consent and privacy policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConsentPolicy {
    /// Allow observing notifications metadata and summaries.
    pub allow_notifications: bool,
    /// Allow observing screen lock and orientation changes.
    pub allow_screen_state: bool,
    /// Allow observing foreground application usage.
    pub allow_foreground_app: bool,
    /// Allow observing battery and device charging status.
    pub allow_device_status: bool,
    /// Allow observing clipboard text previews.
    pub allow_clipboard: bool,
    /// Allow observing media playback metadata.
    pub allow_media_state: bool,
    /// Strictly sandboxed: Raw video access is false by default.
    pub allow_raw_video: bool,
}

impl Default for AgentConsentPolicy {
    fn default() -> Self {
        Self {
            allow_notifications: true,
            allow_screen_state: true,
            allow_foreground_app: true,
            allow_device_status: true,
            allow_clipboard: false, // Strict default: clipboard requires explicit opt-in
            allow_media_state: true,
            allow_raw_video: false, // Strict default: raw video is sandboxed by default
        }
    }
}

impl AgentConsentPolicy {
    /// Returns true if the policy permits observing events in the given category.
    pub fn can_observe(&self, category: AmbientEventCategory) -> bool {
        match category {
            AmbientEventCategory::Notifications => self.allow_notifications,
            AmbientEventCategory::ScreenState => self.allow_screen_state,
            AmbientEventCategory::ForegroundApp => self.allow_foreground_app,
            AmbientEventCategory::DeviceStatus => self.allow_device_status,
            AmbientEventCategory::Clipboard => self.allow_clipboard,
            AmbientEventCategory::MediaState => self.allow_media_state,
        }
    }

    /// Filters an incoming ambient event against this consent policy.
    /// Returns `Some(event)` if permitted, or `None` if blocked by policy.
    pub fn filter_event(&self, event: &AmbientEvent) -> Option<AmbientEvent> {
        if self.can_observe(event.category) {
            Some(event.clone())
        } else {
            None
        }
    }

    /// Binary wire encoding for consent policy sync.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.push(self.allow_notifications as u8);
        buf.push(self.allow_screen_state as u8);
        buf.push(self.allow_foreground_app as u8);
        buf.push(self.allow_device_status as u8);
        buf.push(self.allow_clipboard as u8);
        buf.push(self.allow_media_state as u8);
        buf.push(self.allow_raw_video as u8);
    }

    /// Binary wire decoding for consent policy sync.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < 7 {
            return Err(anyhow!(
                "consent policy payload too short: {} bytes",
                buf.len()
            ));
        }
        Ok(Self {
            allow_notifications: buf[0] != 0,
            allow_screen_state: buf[1] != 0,
            allow_foreground_app: buf[2] != 0,
            allow_device_status: buf[3] != 0,
            allow_clipboard: buf[4] != 0,
            allow_media_state: buf[5] != 0,
            allow_raw_video: buf[6] != 0,
        })
    }
}

/// Agent query request asking for ambient timeline analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQueryRequest {
    /// Unique correlation ID for the query.
    pub query_id: u64,
    /// Client/agent identifier.
    pub agent_id: String,
    /// Natural language question (e.g. "What happened on my phone in the last hour?").
    pub query_text: String,
    /// Start of analysis time window in microseconds UTC.
    pub time_window_start_us: u64,
    /// End of analysis time window in microseconds UTC.
    pub time_window_end_us: u64,
}

impl AgentQueryRequest {
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(&self.query_id.to_be_bytes());

        let agent_bytes = self.agent_id.as_bytes();
        buf.extend_from_slice(&(agent_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(agent_bytes);

        let query_bytes = self.query_text.as_bytes();
        buf.extend_from_slice(&(query_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(query_bytes);

        buf.extend_from_slice(&self.time_window_start_us.to_be_bytes());
        buf.extend_from_slice(&self.time_window_end_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(mut buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 + 2 + 2 + 8 + 8 {
            return Err(anyhow!("query request payload too short"));
        }
        let query_id = u64::from_be_bytes(buf[..8].try_into()?);
        buf = &buf[8..];

        let agent_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < agent_len {
            return Err(anyhow!("truncated agent id in query request"));
        }
        let agent_id = std::str::from_utf8(&buf[..agent_len])?.to_string();
        buf = &buf[agent_len..];

        if buf.len() < 2 + 8 + 8 {
            return Err(anyhow!("query request truncated before query text"));
        }
        let query_len = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < query_len {
            return Err(anyhow!("truncated query text in query request"));
        }
        let query_text = std::str::from_utf8(&buf[..query_len])?.to_string();
        buf = &buf[query_len..];

        if buf.len() < 8 + 8 {
            return Err(anyhow!("query request truncated before time window"));
        }
        let time_window_start_us = u64::from_be_bytes(buf[..8].try_into()?);
        buf = &buf[8..];
        let time_window_end_us = u64::from_be_bytes(buf[..8].try_into()?);

        Ok(Self {
            query_id,
            agent_id,
            query_text,
            time_window_start_us,
            time_window_end_us,
        })
    }
}

/// Agent query response containing synthesized answer and source citations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQueryResponse {
    /// Correlation ID matching the request.
    pub query_id: u64,
    /// Generated structured summary answer text.
    pub answer_text: String,
    /// IDs of ambient events cited in the generated answer.
    pub matched_event_ids: Vec<u64>,
    /// UTC timestamp of response generation in microseconds.
    pub generated_timestamp_us: u64,
}

impl AgentQueryResponse {
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(&self.query_id.to_be_bytes());

        let answer_bytes = self.answer_text.as_bytes();
        buf.extend_from_slice(&(answer_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(answer_bytes);

        buf.extend_from_slice(&(self.matched_event_ids.len() as u16).to_be_bytes());
        for id in &self.matched_event_ids {
            buf.extend_from_slice(&id.to_be_bytes());
        }
        buf.extend_from_slice(&self.generated_timestamp_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(mut buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 + 4 + 2 + 8 {
            return Err(anyhow!("query response payload too short"));
        }
        let query_id = u64::from_be_bytes(buf[..8].try_into()?);
        buf = &buf[8..];

        let answer_len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
        buf = &buf[4..];
        if buf.len() < answer_len {
            return Err(anyhow!("truncated answer text in query response"));
        }
        let answer_text = std::str::from_utf8(&buf[..answer_len])?.to_string();
        buf = &buf[answer_len..];

        if buf.len() < 2 {
            return Err(anyhow!("truncated citation count in query response"));
        }
        let id_count = u16::from_be_bytes(buf[..2].try_into()?) as usize;
        buf = &buf[2..];
        if buf.len() < id_count * 8 + 8 {
            return Err(anyhow!("truncated event citations in query response"));
        }
        let mut matched_event_ids = Vec::with_capacity(id_count);
        for _ in 0..id_count {
            matched_event_ids.push(u64::from_be_bytes(buf[..8].try_into()?));
            buf = &buf[8..];
        }
        let generated_timestamp_us = u64::from_be_bytes(buf[..8].try_into()?);

        Ok(Self {
            query_id,
            answer_text,
            matched_event_ids,
            generated_timestamp_us,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ambient_event_encode_decode() {
        let event = AmbientEvent::new(
            1001,
            AmbientEventCategory::Notifications,
            "phone:GalaxyS24",
            1710000000000000,
            "Slack: Alice mentioned you in #general",
            r#"{"app":"com.slack","title":"Alice","body":"Hey Arjun!","unread":true}"#,
        )
        .unwrap();

        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();

        let decoded = AmbientEvent::decode(&buf).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn test_oversize_ambient_event_rejected() {
        let oversize_meta = "x".repeat(MAX_METADATA_JSON_LEN + 1);
        let res = AmbientEvent::new(
            1002,
            AmbientEventCategory::DeviceStatus,
            "phone:GalaxyS24",
            1710000000000000,
            "Battery Status",
            oversize_meta,
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_agent_consent_policy_filtering() {
        let policy = AgentConsentPolicy {
            allow_notifications: true,
            allow_screen_state: true,
            allow_foreground_app: false, // Blocked
            allow_device_status: true,
            allow_clipboard: false, // Blocked
            allow_media_state: true,
            allow_raw_video: false, // Sandboxed
        };

        let notif_event = AmbientEvent::new(
            1,
            AmbientEventCategory::Notifications,
            "phone:GalaxyS24",
            100,
            "New SMS",
            "{}",
        )
        .unwrap();
        assert!(policy.filter_event(&notif_event).is_some());

        let clip_event = AmbientEvent::new(
            2,
            AmbientEventCategory::Clipboard,
            "phone:GalaxyS24",
            200,
            "Copied password",
            "{}",
        )
        .unwrap();
        assert!(policy.filter_event(&clip_event).is_none());

        let app_event = AmbientEvent::new(
            3,
            AmbientEventCategory::ForegroundApp,
            "phone:GalaxyS24",
            300,
            "Opened Banking App",
            "{}",
        )
        .unwrap();
        assert!(policy.filter_event(&app_event).is_none());
    }

    #[test]
    fn test_agent_query_request_response_round_trip() {
        let req = AgentQueryRequest {
            query_id: 42,
            agent_id: "agent_alpha".to_string(),
            query_text: "What happened on my phone in the last hour?".to_string(),
            time_window_start_us: 1000,
            time_window_end_us: 2000,
        };

        let mut req_buf = Vec::new();
        req.encode(&mut req_buf).unwrap();
        let decoded_req = AgentQueryRequest::decode(&req_buf).unwrap();
        assert_eq!(req, decoded_req);

        let resp = AgentQueryResponse {
            query_id: 42,
            answer_text: "You received 2 Slack notifications and listened to Podcast #101."
                .to_string(),
            matched_event_ids: vec![101, 102, 103],
            generated_timestamp_us: 2050,
        };

        let mut resp_buf = Vec::new();
        resp.encode(&mut resp_buf).unwrap();
        let decoded_resp = AgentQueryResponse::decode(&resp_buf).unwrap();
        assert_eq!(resp, decoded_resp);
    }
}
