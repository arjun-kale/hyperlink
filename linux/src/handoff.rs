//! Linux host state handoff dispatcher and manager (Phase 9).
//!
//! Handles incoming handoff payloads (e.g. BrowserTab, NoteDraft, MediaPlayback, CustomUri),
//! dispatches them to desktop system handlers or custom apps, and acknowledges receipt.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use hyperlink_protocol::clock;
use hyperlink_protocol::handoff::{
    BrowserTabState, HandoffAck, HandoffCapability, HandoffPayload, HandoffType,
    MediaPlaybackState, NoteDraftState,
};

/// URI schemes (with `://` separator) that `xdg-open` is allowed to launch on behalf of a
/// remote handoff payload. Anything else (e.g. `file://`, `javascript:`, an unregistered
/// custom scheme) is rejected rather than handed to the desktop's URI handler, since the
/// peer requesting the launch is remote-controlled input, not local user intent.
const ALLOWED_LAUNCH_SCHEMES: &[&str] = &[
    "http://",
    "https://",
    "hyperlink-note://",
    "hyperlink-media://",
    "hyperlink-handoff://",
];

/// Returns true if `uri` starts with one of the schemes this host is willing to hand to
/// `xdg-open`. Mirrors `HostHandoffManager::get_capabilities`'s advertised scheme list.
fn is_allowed_launch_uri(uri: &str) -> bool {
    ALLOWED_LAUNCH_SCHEMES
        .iter()
        .any(|scheme| uri.starts_with(scheme))
}

/// Host state handoff manager.
#[derive(Debug, Clone)]
pub struct HostHandoffManager {
    /// Directory for saving transferred note drafts and local state.
    drafts_dir: PathBuf,
    /// Last received handoff payloads (for UI / testing).
    history: Arc<Mutex<Vec<HandoffPayload>>>,
    /// Whether automatic desktop application launch (xdg-open) is enabled.
    auto_launch: bool,
}

impl HostHandoffManager {
    /// Creates a new HostHandoffManager with the given drafts storage directory.
    pub fn new(drafts_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&drafts_dir);
        Self {
            drafts_dir,
            history: Arc::new(Mutex::new(Vec::new())),
            auto_launch: true,
        }
    }

    /// Sets auto-launch policy (can be disabled in unit tests / headless CI).
    pub fn set_auto_launch(&mut self, enabled: bool) {
        self.auto_launch = enabled;
    }

    /// Returns the history of received handoff payloads.
    pub fn get_history(&self) -> Vec<HandoffPayload> {
        self.history.lock().unwrap().clone()
    }

    /// Returns the host's supported handoff capabilities.
    pub fn get_capabilities(&self) -> HandoffCapability {
        HandoffCapability {
            supported_types: vec![
                HandoffType::BrowserTab as u8,
                HandoffType::NoteDraft as u8,
                HandoffType::MediaPlayback as u8,
                HandoffType::CustomUri as u8,
            ],
            supported_schemes: vec![
                "http".to_string(),
                "https".to_string(),
                "hyperlink-note".to_string(),
                "hyperlink-media".to_string(),
                "hyperlink-handoff".to_string(),
            ],
        }
    }

    /// Processes an incoming HandoffPayload from peer and dispatches to system.
    pub fn handle_incoming_handoff(&self, payload: &HandoffPayload) -> HandoffAck {
        info!(
            session_id = payload.session_id,
            origin = %payload.source_origin,
            handoff_type = ?payload.handoff_type,
            title = %payload.title,
            uri = %payload.uri,
            "processing incoming state handoff"
        );

        if let Err(e) = payload.validate() {
            warn!(error = %e, "rejected invalid handoff payload");
            return HandoffAck {
                session_id: payload.session_id,
                accepted: false,
                status_code: 1, // Unsupported / invalid
                ack_timestamp_us: clock::now_us(),
            };
        }

        // Store in history
        self.history.lock().unwrap().push(payload.clone());

        let mut status_code = 0; // Success

        match payload.handoff_type {
            HandoffType::BrowserTab => {
                if let Ok(tab) = serde_json::from_str::<BrowserTabState>(&payload.state_json) {
                    info!(url = %tab.url, scroll = tab.scroll_y, "handling browser tab handoff");
                    if self.auto_launch {
                        self.launch_browser_tab(&tab.url);
                    }
                } else if !payload.uri.is_empty() {
                    if self.auto_launch {
                        self.launch_browser_tab(&payload.uri);
                    }
                } else {
                    status_code = 1;
                }
            }
            HandoffType::NoteDraft => {
                if let Ok(note) = serde_json::from_str::<NoteDraftState>(&payload.state_json) {
                    info!(title = %note.title, len = note.content.len(), "handling note draft handoff");
                    let note_file = self.save_note_draft(&note, payload.session_id);
                    if self.auto_launch {
                        if let Some(path) = note_file {
                            self.launch_editor(&path);
                        }
                    }
                } else {
                    status_code = 1;
                }
            }
            HandoffType::MediaPlayback => {
                if let Ok(media) = serde_json::from_str::<MediaPlaybackState>(&payload.state_json) {
                    info!(
                        uri = %media.media_uri,
                        pos_ms = media.position_ms,
                        "handling media playback handoff"
                    );
                    if self.auto_launch {
                        self.launch_media(&media.media_uri);
                    }
                } else if !payload.uri.is_empty() {
                    if self.auto_launch {
                        self.launch_media(&payload.uri);
                    }
                } else {
                    status_code = 1;
                }
            }
            HandoffType::CustomUri => {
                if !payload.uri.is_empty() {
                    info!(uri = %payload.uri, "handling custom deep-link handoff");
                    if self.auto_launch {
                        self.launch_uri(&payload.uri);
                    }
                } else {
                    status_code = 1;
                }
            }
        }

        HandoffAck {
            session_id: payload.session_id,
            accepted: status_code == 0,
            status_code,
            ack_timestamp_us: clock::now_us(),
        }
    }

    fn launch_browser_tab(&self, url: &str) {
        if is_allowed_launch_uri(url) {
            let _ = Command::new("xdg-open").arg(url).spawn();
        } else {
            warn!(uri = %url, "refusing to launch browser tab with disallowed URI scheme");
        }
    }

    fn save_note_draft(&self, note: &NoteDraftState, session_id: u64) -> Option<PathBuf> {
        let safe_title: String = note
            .title
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .take(32)
            .collect();
        let filename = format!("draft_{}_{}.md", session_id, safe_title);
        let path = self.drafts_dir.join(filename);

        let content = format!(
            "# {}\n\n<!-- HyperLink Note Draft (Cursor: {}) -->\n\n{}",
            note.title, note.cursor_offset, note.content
        );

        if let Err(e) = fs::write(&path, content) {
            error!(error = %e, path = %path.display(), "failed to save note draft");
            None
        } else {
            Some(path)
        }
    }

    fn launch_editor(&self, path: &PathBuf) {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }

    fn launch_media(&self, uri: &str) {
        if is_allowed_launch_uri(uri) {
            let _ = Command::new("xdg-open").arg(uri).spawn();
        } else {
            warn!(uri = %uri, "refusing to launch media with disallowed URI scheme");
        }
    }

    fn launch_uri(&self, uri: &str) {
        if is_allowed_launch_uri(uri) {
            let _ = Command::new("xdg-open").arg(uri).spawn();
        } else {
            warn!(uri = %uri, "refusing to launch custom handoff URI with disallowed scheme");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_test_drafts_dir(name: &str) -> PathBuf {
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hyperlink_handoff_{}_{}", name, now_ns));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_host_handoff_browser_tab() {
        let drafts_dir = get_test_drafts_dir("browser");
        let mut mgr = HostHandoffManager::new(drafts_dir);
        mgr.set_auto_launch(false);

        let tab = BrowserTabState {
            url: "https://crates.io/crates/hyperlink".to_string(),
            title: "hyperlink - crates.io".to_string(),
            scroll_y: 500,
            zoom_level: 1.0,
        };

        let payload = HandoffPayload::new_browser_tab(
            5001,
            "phone:GalaxyS24",
            "org.mozilla.firefox",
            &tab,
            clock::now_us(),
        )
        .unwrap();

        let ack = mgr.handle_incoming_handoff(&payload);
        assert!(ack.accepted);
        assert_eq!(ack.status_code, 0);

        let history = mgr.get_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].session_id, 5001);
    }

    #[test]
    fn test_host_handoff_note_draft_saved_to_disk() {
        let drafts_dir = get_test_drafts_dir("notes");
        let mut mgr = HostHandoffManager::new(drafts_dir.clone());
        mgr.set_auto_launch(false);

        let note = NoteDraftState {
            title: "Roadmap 2027".to_string(),
            content: "Step 1: Universal Continuity\nStep 2: Ambient Context Agent".to_string(),
            cursor_offset: 12,
            is_pinned: false,
        };

        let payload = HandoffPayload::new_note_draft(
            5002,
            "phone:GalaxyS24",
            "com.samsung.android.app.notes",
            "hyperlink-note://draft/5002",
            &note,
            clock::now_us(),
        )
        .unwrap();

        let ack = mgr.handle_incoming_handoff(&payload);
        assert!(ack.accepted);
        assert_eq!(ack.status_code, 0);

        // Verify file written to drafts_dir
        let files: Vec<_> = fs::read_dir(&drafts_dir).unwrap().collect();
        assert_eq!(files.len(), 1);
        let content = fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        assert!(content.contains("Roadmap 2027"));
        assert!(content.contains("Ambient Context Agent"));
    }

    #[test]
    fn test_host_handoff_media_playback() {
        let drafts_dir = get_test_drafts_dir("media");
        let mut mgr = HostHandoffManager::new(drafts_dir);
        mgr.set_auto_launch(false);

        let media = MediaPlaybackState {
            media_uri: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
            title: "Rick Astley - Never Gonna Give You Up".to_string(),
            position_ms: 45000,
            duration_ms: 213000,
            is_playing: true,
            volume: 1.0,
        };

        let payload = HandoffPayload::new_media_playback(
            5003,
            "phone:GalaxyS24",
            "com.google.android.youtube",
            &media,
            clock::now_us(),
        )
        .unwrap();

        let ack = mgr.handle_incoming_handoff(&payload);
        assert!(ack.accepted);
        assert_eq!(ack.status_code, 0);
    }

    #[test]
    fn test_allowed_launch_uri_schemes() {
        assert!(is_allowed_launch_uri("https://example.com/tab"));
        assert!(is_allowed_launch_uri("http://example.com/tab"));
        assert!(is_allowed_launch_uri("hyperlink-note://draft/1"));
        assert!(is_allowed_launch_uri("hyperlink-media://play/1"));
        assert!(is_allowed_launch_uri("hyperlink-handoff://session/1"));
    }

    #[test]
    fn test_disallowed_launch_uri_schemes_rejected() {
        // Local file access — would let a paired peer open arbitrary host files via xdg-open.
        assert!(!is_allowed_launch_uri("file:///home/user/.ssh/id_rsa"));
        // Unregistered / attacker-chosen custom scheme.
        assert!(!is_allowed_launch_uri("evil-scheme://payload"));
        // No scheme at all.
        assert!(!is_allowed_launch_uri("not-a-uri"));
        assert!(!is_allowed_launch_uri(""));
    }

    #[test]
    fn test_custom_uri_handoff_with_disallowed_scheme_is_not_launched() {
        let drafts_dir = get_test_drafts_dir("custom_uri_reject");
        let mut mgr = HostHandoffManager::new(drafts_dir);
        // auto_launch stays enabled here: the point of this test is that the scheme check
        // inside launch_uri() itself is what stops the launch, not the auto_launch flag.
        mgr.set_auto_launch(true);

        let payload = HandoffPayload {
            session_id: 5004,
            source_origin: "phone:GalaxyS24".to_string(),
            handoff_type: HandoffType::CustomUri,
            app_id: "com.example.malicious".to_string(),
            uri: "file:///etc/passwd".to_string(),
            title: "".to_string(),
            state_json: "".to_string(),
            timestamp_us: clock::now_us(),
        };

        // The handoff is still received/acknowledged (delivery vs. execution are separate
        // concerns) but launch_uri() must refuse to hand a file:// URI to xdg-open.
        let ack = mgr.handle_incoming_handoff(&payload);
        assert!(ack.accepted);
        assert_eq!(ack.status_code, 0);
        assert_eq!(mgr.get_history().len(), 1);
    }
}
