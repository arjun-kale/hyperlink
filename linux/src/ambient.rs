//! Ambient Context Event Bus and Reference Agent for Linux Host (Phase 10).
//!
//! Provides a local, privacy-gated event bus storing device telemetry and notification/clipboard
//! metadata, and a desktop agent query engine answering natural language questions like
//! "what happened on my phone in the last hour?".

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::{debug, info};

use hyperlink_protocol::ambient::{
    AgentConsentPolicy, AgentQueryRequest, AgentQueryResponse, AmbientEvent, AmbientEventCategory,
};
use hyperlink_protocol::clock;

/// In-memory ring buffer maximum capacity for ambient events.
const RING_BUFFER_CAPACITY: usize = 10_000;
/// Broadcast channel buffer capacity.
const BROADCAST_CAPACITY: usize = 1024;

/// Internal storage for the ambient event bus.
struct BusState {
    events: VecDeque<AmbientEvent>,
    journal_path: Option<PathBuf>,
}

/// The Linux Host Ambient Context Event Bus.
#[derive(Clone)]
pub struct AmbientEventBus {
    state: Arc<Mutex<BusState>>,
    broadcast_tx: broadcast::Sender<AmbientEvent>,
}

impl AmbientEventBus {
    /// Creates a new AmbientEventBus, optionally appending to a journal file.
    pub fn new(journal_path: Option<PathBuf>) -> Self {
        if let Some(ref path) = journal_path {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
        }

        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            state: Arc::new(Mutex::new(BusState {
                events: VecDeque::with_capacity(RING_BUFFER_CAPACITY),
                journal_path,
            })),
            broadcast_tx,
        }
    }

    /// Publishes a new ambient context event onto the bus.
    pub fn publish(&self, event: AmbientEvent) -> anyhow::Result<()> {
        event.validate()?;

        debug!(
            id = event.event_id,
            category = ?event.category,
            source = %event.source,
            summary = %event.summary,
            "publishing ambient event"
        );

        // Store in in-memory ring buffer and optionally write to disk
        {
            let mut state = self.state.lock().unwrap();
            if state.events.len() >= RING_BUFFER_CAPACITY {
                state.events.pop_front();
            }
            state.events.push_back(event.clone());

            if let Some(ref path) = state.journal_path {
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                    if let Ok(json) = serde_json::to_string(&event) {
                        let _ = writeln!(file, "{}", json);
                    }
                }
            }
        }

        // Broadcast to live subscribers (ignore error if no receivers active)
        let _ = self.broadcast_tx.send(event);
        Ok(())
    }

    /// Subscribes to real-time ambient events on the bus.
    pub fn subscribe(&self) -> broadcast::Receiver<AmbientEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Queries historical ambient events within the specified time window, filtered by consent policy.
    pub fn query(
        &self,
        window_start_us: u64,
        window_end_us: u64,
        policy: &AgentConsentPolicy,
    ) -> Vec<AmbientEvent> {
        let state = self.state.lock().unwrap();
        state
            .events
            .iter()
            .filter(|e| {
                e.timestamp_us >= window_start_us
                    && e.timestamp_us <= window_end_us
                    && policy.can_observe(e.category)
            })
            .cloned()
            .collect()
    }

    /// Returns the count of events currently in the ring buffer.
    pub fn count(&self) -> usize {
        self.state.lock().unwrap().events.len()
    }
}

/// Ambient Context Agent for querying and reasoning over phone telemetry.
pub struct AmbientContextAgent {
    bus: AmbientEventBus,
    consent_policy: AgentConsentPolicy,
}

impl AmbientContextAgent {
    /// Creates a new AmbientContextAgent with the given bus and consent policy.
    pub fn new(bus: AmbientEventBus, consent_policy: AgentConsentPolicy) -> Self {
        Self {
            bus,
            consent_policy,
        }
    }

    /// Returns a reference to the active consent policy.
    pub fn consent_policy(&self) -> &AgentConsentPolicy {
        &self.consent_policy
    }

    /// Updates the active consent policy.
    pub fn set_consent_policy(&mut self, policy: AgentConsentPolicy) {
        self.consent_policy = policy;
    }

    /// Processes an agent query request and synthesizes a structured timeline answer.
    pub fn process_query(&self, req: &AgentQueryRequest) -> AgentQueryResponse {
        info!(
            agent = %req.agent_id,
            query = %req.query_text,
            start_us = req.time_window_start_us,
            end_us = req.time_window_end_us,
            "processing agent query"
        );

        let events = self.bus.query(
            req.time_window_start_us,
            req.time_window_end_us,
            &self.consent_policy,
        );

        let mut matched_event_ids = Vec::new();
        let mut notif_summaries = Vec::new();
        let mut app_summaries = Vec::new();
        let mut media_summaries = Vec::new();
        let mut status_summaries = Vec::new();
        let mut clip_summaries = Vec::new();

        for event in &events {
            matched_event_ids.push(event.event_id);
            match event.category {
                AmbientEventCategory::Notifications => {
                    notif_summaries.push(format!("• [#{}] {}", event.event_id, event.summary));
                }
                AmbientEventCategory::ForegroundApp => {
                    app_summaries.push(format!("• [#{}] {}", event.event_id, event.summary));
                }
                AmbientEventCategory::MediaState => {
                    media_summaries.push(format!("• [#{}] {}", event.event_id, event.summary));
                }
                AmbientEventCategory::DeviceStatus => {
                    status_summaries.push(format!("• [#{}] {}", event.event_id, event.summary));
                }
                AmbientEventCategory::Clipboard => {
                    clip_summaries.push(format!("• [#{}] {}", event.event_id, event.summary));
                }
                AmbientEventCategory::ScreenState => {}
            }
        }

        let mut answer_lines = Vec::new();
        let duration_mins = (req
            .time_window_end_us
            .saturating_sub(req.time_window_start_us))
            / 60_000_000;
        answer_lines.push(format!(
            "Summary of phone activity over the past {} minute(s) ({} total events observed):",
            duration_mins.max(1),
            events.len()
        ));

        if !notif_summaries.is_empty() {
            answer_lines.push(format!("\nNotifications ({}):", notif_summaries.len()));
            answer_lines.extend(notif_summaries);
        }

        if !app_summaries.is_empty() {
            answer_lines.push(format!("\nApps Used ({}):", app_summaries.len()));
            answer_lines.extend(app_summaries);
        }

        if !media_summaries.is_empty() {
            answer_lines.push(format!("\nMedia Playback ({}):", media_summaries.len()));
            answer_lines.extend(media_summaries);
        }

        if !clip_summaries.is_empty() {
            answer_lines.push(format!("\nClipboard ({}):", clip_summaries.len()));
            answer_lines.extend(clip_summaries);
        }

        if !status_summaries.is_empty() {
            answer_lines.push(format!("\nDevice Telemetry ({}):", status_summaries.len()));
            answer_lines.extend(status_summaries);
        }

        if events.is_empty() {
            answer_lines.push("No activity recorded within the requested time window or permitted consent categories.".to_string());
        }

        AgentQueryResponse {
            query_id: req.query_id,
            answer_text: answer_lines.join("\n"),
            matched_event_ids,
            generated_timestamp_us: clock::now_us(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ambient_bus_publish_and_query() {
        let bus = AmbientEventBus::new(None);

        let e1 = AmbientEvent::new(
            1,
            AmbientEventCategory::Notifications,
            "phone:GalaxyS24",
            1000,
            "Received WhatsApp message from Mom",
            "{}",
        )
        .unwrap();

        let e2 = AmbientEvent::new(
            2,
            AmbientEventCategory::ForegroundApp,
            "phone:GalaxyS24",
            2000,
            "Switched to YouTube Music",
            "{}",
        )
        .unwrap();

        bus.publish(e1).unwrap();
        bus.publish(e2).unwrap();

        assert_eq!(bus.count(), 2);

        let policy = AgentConsentPolicy::default();
        let results = bus.query(500, 2500, &policy);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_consent_gating_blocks_unconsented_categories() {
        let bus = AmbientEventBus::new(None);

        let e_notif = AmbientEvent::new(
            1,
            AmbientEventCategory::Notifications,
            "phone:GalaxyS24",
            1000,
            "Incoming SMS",
            "{}",
        )
        .unwrap();

        let e_clip = AmbientEvent::new(
            2,
            AmbientEventCategory::Clipboard,
            "phone:GalaxyS24",
            1100,
            "Copied credit card number",
            "{}",
        )
        .unwrap();

        bus.publish(e_notif).unwrap();
        bus.publish(e_clip).unwrap();

        // Default policy blocks clipboard
        let policy = AgentConsentPolicy {
            allow_notifications: true,
            allow_clipboard: false, // Strict block
            ..Default::default()
        };

        let results = bus.query(500, 2000, &policy);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].category, AmbientEventCategory::Notifications);
    }

    #[test]
    fn test_raw_video_sandboxing_invariant() {
        let policy = AgentConsentPolicy::default();
        // Strict invariant: raw video is sandboxed
        assert!(!policy.allow_raw_video);
    }

    #[test]
    fn test_ambient_agent_last_hour_query_synthesis() {
        let bus = AmbientEventBus::new(None);

        let e1 = AmbientEvent::new(
            101,
            AmbientEventCategory::Notifications,
            "phone:GalaxyS24",
            10_000_000,
            "Slack: Bob requested code review on PR #42",
            "{}",
        )
        .unwrap();

        let e2 = AmbientEvent::new(
            102,
            AmbientEventCategory::MediaState,
            "phone:GalaxyS24",
            20_000_000,
            "Playing: Daft Punk - Harder, Better, Faster, Stronger",
            "{}",
        )
        .unwrap();

        bus.publish(e1).unwrap();
        bus.publish(e2).unwrap();

        let agent = AmbientContextAgent::new(bus, AgentConsentPolicy::default());

        let req = AgentQueryRequest {
            query_id: 1,
            agent_id: "test_agent".to_string(),
            query_text: "What happened on my phone in the last hour?".to_string(),
            time_window_start_us: 0,
            time_window_end_us: 60_000_000,
        };

        let resp = agent.process_query(&req);
        assert_eq!(resp.query_id, 1);
        assert_eq!(resp.matched_event_ids, vec![101, 102]);
        assert!(resp.answer_text.contains("Notifications"));
        assert!(resp.answer_text.contains("PR #42"));
        assert!(resp.answer_text.contains("Daft Punk"));
    }
}
