//! Message type definitions for the HyperLink protocol.
//!
//! Every wire message is tagged with a `MessageType` discriminant in the header.
//! Phase 0 defines clock-sync and echo types used by the bench harness;
//! subsequent phases extend this enum with video, input, control-plane, and
//! file-transfer types.

/// Discriminant for each message type on the wire.
///
/// Encoded as a single `u8` in the header. Values are assigned explicitly
/// (not auto-numbered) so wire compatibility survives reordering in source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MessageType {
    // --- Phase 0: Bench harness ---
    /// NTP-style clock synchronization request (client → server).
    ClockSyncRequest = 0x01,
    /// NTP-style clock synchronization response (server → client).
    ClockSyncResponse = 0x02,
    /// Echo request carrying a sequence number and timestamp (client → server).
    EchoRequest = 0x10,
    /// Echo response mirroring the request (server → client).
    EchoResponse = 0x11,
    /// Keepalive heartbeat (bidirectional).
    Heartbeat = 0x20,
    // --- Phase 2: video stream messages (0x30–0x3F) ---
    /// Encoded H.264 video frame (phone → host, unreliable datagram).
    VideoFrame = 0x30,
    /// Codec configuration: SPS/PPS + encoder params (phone → host, reliable).
    VideoConfig = 0x31,
    /// Receiver bitrate feedback (host → phone, reliable).
    BitrateAck = 0x32,
    // --- Phase 3: input stream messages (0x40–0x4F) ---
    /// Pointer movement / click / touch event (host → phone, reliable).
    PointerEvent = 0x40,
    /// Keyboard key press / release event (host → phone, reliable).
    KeyEvent = 0x41,
    /// Mouse wheel / scroll gesture delta (host → phone, reliable).
    ScrollEvent = 0x42,
    /// Android system navigation action: Back, Home, Recents (host → phone, reliable).
    NavEvent = 0x43,
    /// Input round-trip latency acknowledgement (phone → host, reliable).
    InputAck = 0x44,
    // 0x50–0x5F: control-plane messages (notifications, clipboard)
    /// Client initiates pairing and sends its name (client → server).
    PairRequest = 0x50,
    /// Server accepts or rejects pairing (server → client).
    PairResponse = 0x51,
    // --- Phase 4: notification stream messages (0x60–0x6F) ---
    /// Notification posted on phone (phone → host, reliable).
    NotificationPost = 0x60,
    /// Notification dismissed on phone or host (bidirectional, reliable).
    NotificationDismiss = 0x61,
    /// Action button clicked on host notification (host → phone, reliable).
    NotificationActionInvoke = 0x62,
    /// Do-Not-Disturb state synchronization (bidirectional, reliable).
    DndSync = 0x63,
    /// Notification transit latency acknowledgement (host → phone, reliable).
    NotificationAck = 0x64,
    // --- Phase 5: clipboard stream messages (0x70–0x7F) ---
    /// Clipboard content synchronization (bidirectional, reliable).
    ClipboardMessage = 0x70,
    /// Clipboard receipt and write acknowledgement (bidirectional, reliable).
    ClipboardAck = 0x71,
    // --- Phase 6: file access stream messages (0x80–0x8F) ---
    /// Stat a file or directory path (host → phone, reliable).
    FileStatRequest = 0x80,
    /// Stat response returning entry metadata (phone → host, reliable).
    FileStatResponse = 0x81,
    /// List directory contents (host → phone, reliable).
    FileListRequest = 0x82,
    /// List directory response returning children entries (phone → host, reliable).
    FileListResponse = 0x83,
    /// Lazy chunked read from a path at offset (host → phone, reliable).
    FileReadChunkRequest = 0x84,
    /// Chunked read response carrying requested slice of bytes (phone → host, reliable).
    FileReadChunkResponse = 0x85,
    /// Write chunk back to phone storage at offset (host → phone, reliable).
    FileWriteChunkRequest = 0x86,
    /// Write chunk response confirming write status (phone → host, reliable).
    FileWriteChunkResponse = 0x87,
    /// Thumbnail request for image/video media preview (host → phone, reliable).
    FileThumbnailRequest = 0x88,
    /// Thumbnail response with downscaled image bytes (phone → host, reliable).
    FileThumbnailResponse = 0x89,
    // --- Phase 7: network resilience & multipath messages (0x90–0x9F) ---
    /// Probe request for latency and health verification on a path (bidirectional, reliable).
    PathProbeRequest = 0x90,
    /// Probe response carrying return timestamp for path RTT calculation (bidirectional, reliable).
    PathProbeResponse = 0x91,
    /// Explicit notice informing peer of path switch / failover transition (bidirectional, reliable).
    PathSwitchNotice = 0x92,
    /// Acknowledgement of path switch notice (bidirectional, reliable).
    PathSwitchAck = 0x93,
    /// Periodic keepalive heartbeat on a candidate path (bidirectional, reliable).
    PathHeartbeat = 0x94,
    /// Periodic health report summarizing link condition (bidirectional, reliable).
    PathHealthReport = 0x95,
    // --- Phase 8: proximity, pre-warming & workflow state messages (0xA0–0xAF) ---
    /// Proximity beacon carrying ranging metadata (phone → host, reliable or datagram).
    ProximityBeacon = 0xA0,
    /// Proximity acknowledgement acknowledging beacon receipt (host → phone, reliable).
    ProximityAck = 0xA1,
    /// Pre-warm state synchronization (bidirectional, reliable).
    PreWarmState = 0xA2,
    /// Save current active workflow state (window geometry, active package, orientation) (bidirectional, reliable).
    WorkflowStateSave = 0xA3,
    /// Restore workflow state on connection activate (host → phone, reliable).
    WorkflowStateRestore = 0xA4,
    /// Workflow state acknowledgement (bidirectional, reliable).
    WorkflowStateAck = 0xA5,
    // --- Phase 9: scoped app-state handoff messages (0xB0–0xBF) ---
    /// State handoff payload for continuing task/context on peer (bidirectional, reliable).
    HandoffPayload = 0xB0,
    /// State handoff acknowledgement confirming receipt/launch (bidirectional, reliable).
    HandoffAck = 0xB1,
    /// Discover supported handoff capabilities and schemas on peer (bidirectional, reliable).
    HandoffDiscover = 0xB2,
    /// Discover response listing supported handoff schemas (bidirectional, reliable).
    HandoffDiscoverAck = 0xB3,
    // --- Phase 10: ambient context agent messages (0xC0–0xCF) ---
    /// Ambient telemetry/context event published from peer (phone → host, reliable or datagram).
    AmbientEventPublish = 0xC0,
    /// Acknowledgement of ambient event receipt (host → phone, reliable).
    AmbientEventAck = 0xC1,
    /// Dynamic agent consent policy update (host → phone, reliable).
    AgentConsentUpdate = 0xC2,
    /// Query current agent consent policy (bidirectional, reliable).
    AgentConsentQuery = 0xC3,
    /// Query request from desktop agent (host → agent / engine, reliable).
    AgentQueryRequest = 0xC4,
    /// Structured response from ambient agent containing answer + citations (engine → host, reliable).
    AgentQueryResponse = 0xC5,
}

impl TryFrom<u8> for MessageType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::ClockSyncRequest),
            0x02 => Ok(Self::ClockSyncResponse),
            0x10 => Ok(Self::EchoRequest),
            0x11 => Ok(Self::EchoResponse),
            0x20 => Ok(Self::Heartbeat),
            0x30 => Ok(Self::VideoFrame),
            0x31 => Ok(Self::VideoConfig),
            0x32 => Ok(Self::BitrateAck),
            0x40 => Ok(Self::PointerEvent),
            0x41 => Ok(Self::KeyEvent),
            0x42 => Ok(Self::ScrollEvent),
            0x43 => Ok(Self::NavEvent),
            0x44 => Ok(Self::InputAck),
            0x50 => Ok(Self::PairRequest),
            0x51 => Ok(Self::PairResponse),
            0x60 => Ok(Self::NotificationPost),
            0x61 => Ok(Self::NotificationDismiss),
            0x62 => Ok(Self::NotificationActionInvoke),
            0x63 => Ok(Self::DndSync),
            0x64 => Ok(Self::NotificationAck),
            0x70 => Ok(Self::ClipboardMessage),
            0x71 => Ok(Self::ClipboardAck),
            0x80 => Ok(Self::FileStatRequest),
            0x81 => Ok(Self::FileStatResponse),
            0x82 => Ok(Self::FileListRequest),
            0x83 => Ok(Self::FileListResponse),
            0x84 => Ok(Self::FileReadChunkRequest),
            0x85 => Ok(Self::FileReadChunkResponse),
            0x86 => Ok(Self::FileWriteChunkRequest),
            0x87 => Ok(Self::FileWriteChunkResponse),
            0x88 => Ok(Self::FileThumbnailRequest),
            0x89 => Ok(Self::FileThumbnailResponse),
            0x90 => Ok(Self::PathProbeRequest),
            0x91 => Ok(Self::PathProbeResponse),
            0x92 => Ok(Self::PathSwitchNotice),
            0x93 => Ok(Self::PathSwitchAck),
            0x94 => Ok(Self::PathHeartbeat),
            0x95 => Ok(Self::PathHealthReport),
            0xA0 => Ok(Self::ProximityBeacon),
            0xA1 => Ok(Self::ProximityAck),
            0xA2 => Ok(Self::PreWarmState),
            0xA3 => Ok(Self::WorkflowStateSave),
            0xA4 => Ok(Self::WorkflowStateRestore),
            0xA5 => Ok(Self::WorkflowStateAck),
            0xB0 => Ok(Self::HandoffPayload),
            0xB1 => Ok(Self::HandoffAck),
            0xB2 => Ok(Self::HandoffDiscover),
            0xB3 => Ok(Self::HandoffDiscoverAck),
            0xC0 => Ok(Self::AmbientEventPublish),
            0xC1 => Ok(Self::AmbientEventAck),
            0xC2 => Ok(Self::AgentConsentUpdate),
            0xC3 => Ok(Self::AgentConsentQuery),
            0xC4 => Ok(Self::AgentQueryRequest),
            0xC5 => Ok(Self::AgentQueryResponse),
            other => Err(other),
        }
    }
}

impl std::fmt::Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClockSyncRequest => write!(f, "ClockSyncRequest"),
            Self::ClockSyncResponse => write!(f, "ClockSyncResponse"),
            Self::EchoRequest => write!(f, "EchoRequest"),
            Self::EchoResponse => write!(f, "EchoResponse"),
            Self::Heartbeat => write!(f, "Heartbeat"),
            Self::VideoFrame => write!(f, "VideoFrame"),
            Self::VideoConfig => write!(f, "VideoConfig"),
            Self::BitrateAck => write!(f, "BitrateAck"),
            Self::PointerEvent => write!(f, "PointerEvent"),
            Self::KeyEvent => write!(f, "KeyEvent"),
            Self::ScrollEvent => write!(f, "ScrollEvent"),
            Self::NavEvent => write!(f, "NavEvent"),
            Self::InputAck => write!(f, "InputAck"),
            Self::PairRequest => write!(f, "PairRequest"),
            Self::PairResponse => write!(f, "PairResponse"),
            Self::NotificationPost => write!(f, "NotificationPost"),
            Self::NotificationDismiss => write!(f, "NotificationDismiss"),
            Self::NotificationActionInvoke => write!(f, "NotificationActionInvoke"),
            Self::DndSync => write!(f, "DndSync"),
            Self::NotificationAck => write!(f, "NotificationAck"),
            Self::ClipboardMessage => write!(f, "ClipboardMessage"),
            Self::ClipboardAck => write!(f, "ClipboardAck"),
            Self::FileStatRequest => write!(f, "FileStatRequest"),
            Self::FileStatResponse => write!(f, "FileStatResponse"),
            Self::FileListRequest => write!(f, "FileListRequest"),
            Self::FileListResponse => write!(f, "FileListResponse"),
            Self::FileReadChunkRequest => write!(f, "FileReadChunkRequest"),
            Self::FileReadChunkResponse => write!(f, "FileReadChunkResponse"),
            Self::FileWriteChunkRequest => write!(f, "FileWriteChunkRequest"),
            Self::FileWriteChunkResponse => write!(f, "FileWriteChunkResponse"),
            Self::FileThumbnailRequest => write!(f, "FileThumbnailRequest"),
            Self::FileThumbnailResponse => write!(f, "FileThumbnailResponse"),
            Self::PathProbeRequest => write!(f, "PathProbeRequest"),
            Self::PathProbeResponse => write!(f, "PathProbeResponse"),
            Self::PathSwitchNotice => write!(f, "PathSwitchNotice"),
            Self::PathSwitchAck => write!(f, "PathSwitchAck"),
            Self::PathHeartbeat => write!(f, "PathHeartbeat"),
            Self::PathHealthReport => write!(f, "PathHealthReport"),
            Self::ProximityBeacon => write!(f, "ProximityBeacon"),
            Self::ProximityAck => write!(f, "ProximityAck"),
            Self::PreWarmState => write!(f, "PreWarmState"),
            Self::WorkflowStateSave => write!(f, "WorkflowStateSave"),
            Self::WorkflowStateRestore => write!(f, "WorkflowStateRestore"),
            Self::WorkflowStateAck => write!(f, "WorkflowStateAck"),
            Self::HandoffPayload => write!(f, "HandoffPayload"),
            Self::HandoffAck => write!(f, "HandoffAck"),
            Self::HandoffDiscover => write!(f, "HandoffDiscover"),
            Self::HandoffDiscoverAck => write!(f, "HandoffDiscoverAck"),
            Self::AmbientEventPublish => write!(f, "AmbientEventPublish"),
            Self::AmbientEventAck => write!(f, "AmbientEventAck"),
            Self::AgentConsentUpdate => write!(f, "AgentConsentUpdate"),
            Self::AgentConsentQuery => write!(f, "AgentConsentQuery"),
            Self::AgentQueryRequest => write!(f, "AgentQueryRequest"),
            Self::AgentQueryResponse => write!(f, "AgentQueryResponse"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_types() {
        let types = [
            MessageType::ClockSyncRequest,
            MessageType::ClockSyncResponse,
            MessageType::EchoRequest,
            MessageType::EchoResponse,
            MessageType::Heartbeat,
            MessageType::VideoFrame,
            MessageType::VideoConfig,
            MessageType::BitrateAck,
            MessageType::PointerEvent,
            MessageType::KeyEvent,
            MessageType::ScrollEvent,
            MessageType::NavEvent,
            MessageType::InputAck,
            MessageType::PairRequest,
            MessageType::PairResponse,
            MessageType::NotificationPost,
            MessageType::NotificationDismiss,
            MessageType::NotificationActionInvoke,
            MessageType::DndSync,
            MessageType::NotificationAck,
            MessageType::ClipboardMessage,
            MessageType::ClipboardAck,
            MessageType::FileStatRequest,
            MessageType::FileStatResponse,
            MessageType::FileListRequest,
            MessageType::FileListResponse,
            MessageType::FileReadChunkRequest,
            MessageType::FileReadChunkResponse,
            MessageType::FileWriteChunkRequest,
            MessageType::FileWriteChunkResponse,
            MessageType::FileThumbnailRequest,
            MessageType::FileThumbnailResponse,
            MessageType::PathProbeRequest,
            MessageType::PathProbeResponse,
            MessageType::PathSwitchNotice,
            MessageType::PathSwitchAck,
            MessageType::PathHeartbeat,
            MessageType::PathHealthReport,
            MessageType::ProximityBeacon,
            MessageType::ProximityAck,
            MessageType::PreWarmState,
            MessageType::WorkflowStateSave,
            MessageType::WorkflowStateRestore,
            MessageType::WorkflowStateAck,
            MessageType::HandoffPayload,
            MessageType::HandoffAck,
            MessageType::HandoffDiscover,
            MessageType::HandoffDiscoverAck,
            MessageType::AmbientEventPublish,
            MessageType::AmbientEventAck,
            MessageType::AgentConsentUpdate,
            MessageType::AgentConsentQuery,
            MessageType::AgentQueryRequest,
            MessageType::AgentQueryResponse,
        ];
        for msg_type in types {
            let raw = msg_type as u8;
            let decoded = MessageType::try_from(raw).unwrap();
            assert_eq!(msg_type, decoded);
        }
    }

    #[test]
    fn unknown_type_returns_err() {
        assert!(MessageType::try_from(0xFE).is_err());
    }
}
