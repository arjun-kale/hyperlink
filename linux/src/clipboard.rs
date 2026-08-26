//! Bidirectional Linux clipboard synchronization (Phase 5).
//!
//! Provides clipboard change detection, remote write application,
//! loop-prevention using origin IDs and SHA-256 hashes, and support
//! for both text and large image payloads.

use anyhow::Result;
use hyperlink_protocol::clipboard::{compute_content_hash, ClipboardMessage, ClipboardType};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, info};

/// Content item captured from the local Linux system clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalClipboardItem {
    Text(String),
    Image { mime_type: String, bytes: Vec<u8> },
}

/// Linux clipboard manager tracking origin IDs and deduplicating content hashes with recency ordering.
#[derive(Clone)]
pub struct ClipboardManager {
    origin_id: String,
    recent_hashes: Arc<Mutex<std::collections::VecDeque<[u8; 32]>>>,
    last_local_write_timestamp_us: Arc<AtomicU64>,
    last_applied_remote_timestamp_us: Arc<AtomicU64>,
    last_applied_remote_seq: Arc<AtomicU64>,
    seq: Arc<AtomicU64>,
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardManager {
    pub fn new() -> Self {
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("HOST"))
            .unwrap_or_else(|_| "linux-desktop".to_string());
        let origin_id = format!("host:{}", hostname);

        Self {
            origin_id,
            recent_hashes: Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(32))),
            last_local_write_timestamp_us: Arc::new(AtomicU64::new(0)),
            last_applied_remote_timestamp_us: Arc::new(AtomicU64::new(0)),
            last_applied_remote_seq: Arc::new(AtomicU64::new(0)),
            seq: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn origin_id(&self) -> &str {
        &self.origin_id
    }

    /// Checks if a payload hash matches any recently synced hash (for echo loop prevention).
    pub fn is_echo_loop(&self, remote_origin: &str, content_hash: &[u8; 32]) -> bool {
        if remote_origin == self.origin_id {
            return true;
        }
        let guard = self.recent_hashes.lock().unwrap();
        guard.contains(content_hash)
    }

    /// Sets the synced hash in the recent hash ring buffer to prevent echo loops.
    pub fn record_synced_hash(&self, content_hash: [u8; 32]) {
        let mut guard = self.recent_hashes.lock().unwrap();
        if guard.contains(&content_hash) {
            return;
        }
        if guard.len() >= 32 {
            guard.pop_front();
        }
        guard.push_back(content_hash);
    }

    /// Writes a remote clipboard message to the Linux OS clipboard if fresh and not looped.
    pub fn apply_remote_clip(&self, msg: &ClipboardMessage) -> Result<()> {
        if self.is_echo_loop(&msg.origin_id, &msg.content_hash) {
            debug!("ignoring echo or duplicate clip from {}", msg.origin_id);
            return Ok(());
        }

        // Check timestamp / sequence freshness
        let last_local_ts = self.last_local_write_timestamp_us.load(Ordering::Relaxed);
        if msg.timestamp_us > 0 && msg.timestamp_us < last_local_ts {
            debug!(
                msg_ts = msg.timestamp_us,
                local_ts = last_local_ts,
                "ignoring stale remote clip (older than newer local clip)"
            );
            return Ok(());
        }

        let last_remote_ts = self
            .last_applied_remote_timestamp_us
            .load(Ordering::Relaxed);
        if msg.timestamp_us > 0 && msg.timestamp_us <= last_remote_ts {
            debug!(
                msg_ts = msg.timestamp_us,
                last_remote = last_remote_ts,
                "ignoring out-of-order remote clip"
            );
            return Ok(());
        }

        self.record_synced_hash(msg.content_hash);
        self.last_applied_remote_timestamp_us
            .store(msg.timestamp_us, Ordering::Relaxed);
        self.last_applied_remote_seq
            .store(msg.seq, Ordering::Relaxed);

        match msg.content_type {
            ClipboardType::Text => {
                let text = String::from_utf8_lossy(&msg.payload);
                write_text_to_system(&text)?;
                info!(
                    "applied remote text clip to system clipboard (len={})",
                    text.len()
                );
            }
            ClipboardType::Image => {
                write_image_to_system(&msg.mime_type, &msg.payload)?;
                info!(
                    "applied remote image clip to system clipboard (mime={}, size={})",
                    msg.mime_type,
                    msg.payload.len()
                );
            }
        }
        Ok(())
    }

    /// Creates a ClipboardMessage for a local clipboard item.
    pub fn create_outgoing_message(&self, item: LocalClipboardItem) -> ClipboardMessage {
        let now_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        self.last_local_write_timestamp_us
            .store(now_us, Ordering::Relaxed);

        match item {
            LocalClipboardItem::Text(text) => {
                let msg = ClipboardMessage::new_text(self.origin_id.clone(), &text, seq, now_us);
                self.record_synced_hash(msg.content_hash);
                msg
            }
            LocalClipboardItem::Image { mime_type, bytes } => {
                let msg = ClipboardMessage::new_image(
                    self.origin_id.clone(),
                    mime_type,
                    bytes,
                    seq,
                    now_us,
                );
                self.record_synced_hash(msg.content_hash);
                msg
            }
        }
    }

    /// Starts a background clipboard watcher that polls the system clipboard
    /// and dispatches new items to the outgoing MPSC sender.
    pub fn start_watcher(
        &self,
        tx: mpsc::UnboundedSender<ClipboardMessage>,
    ) -> tokio::task::JoinHandle<()> {
        let mgr = self.clone();
        tokio::spawn(async move {
            info!("started Linux clipboard watcher (polling interval: 300ms)");
            let mut interval = tokio::time::interval(Duration::from_millis(300));
            loop {
                interval.tick().await;
                if let Some(item) = read_system_clipboard() {
                    let hash = match &item {
                        LocalClipboardItem::Text(t) => compute_content_hash(t.as_bytes()),
                        LocalClipboardItem::Image { bytes, .. } => compute_content_hash(bytes),
                    };

                    let is_dup = {
                        let guard = mgr.recent_hashes.lock().unwrap();
                        guard.contains(&hash)
                    };

                    if !is_dup {
                        let msg = mgr.create_outgoing_message(item);
                        debug!("detected new local clipboard item (seq={})", msg.seq);
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                }
            }
        })
    }
}

/// Reads the current clipboard content using wl-paste (Wayland) or xclip (X11).
pub fn read_system_clipboard() -> Option<LocalClipboardItem> {
    // 1. Try wl-paste
    if let Ok(types_output) = Command::new("wl-paste").arg("--list-types").output() {
        if types_output.status.success() {
            let types_str = String::from_utf8_lossy(&types_output.stdout);
            // Check for image types first
            for line in types_str.lines() {
                let t = line.trim();
                if t == "image/png" || t == "image/jpeg" {
                    if let Ok(img_output) = Command::new("wl-paste").args(["--type", t]).output() {
                        if img_output.status.success() && !img_output.stdout.is_empty() {
                            return Some(LocalClipboardItem::Image {
                                mime_type: t.to_string(),
                                bytes: img_output.stdout,
                            });
                        }
                    }
                }
            }
            // Fallback to text
            if let Ok(text_output) = Command::new("wl-paste").arg("--no-newline").output() {
                if text_output.status.success() && !text_output.stdout.is_empty() {
                    if let Ok(text) = String::from_utf8(text_output.stdout) {
                        return Some(LocalClipboardItem::Text(text));
                    }
                }
            }
        }
    }

    // 2. Fallback to xclip
    if let Ok(text_output) = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .output()
    {
        if text_output.status.success() && !text_output.stdout.is_empty() {
            if let Ok(text) = String::from_utf8(text_output.stdout) {
                return Some(LocalClipboardItem::Text(text));
            }
        }
    }

    None
}

/// Writes text to the OS clipboard using wl-copy (Wayland) or xclip (X11).
pub fn write_text_to_system(text: &str) -> Result<()> {
    if let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }

    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-i"])
        .stdin(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    child.wait()?;
    Ok(())
}

/// Writes image bytes to the OS clipboard using wl-copy (Wayland) or xclip (X11).
pub fn write_image_to_system(mime_type: &str, bytes: &[u8]) -> Result<()> {
    if let Ok(mut child) = Command::new("wl-copy")
        .args(["--type", mime_type])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes);
        }
        let _ = child.wait();
        return Ok(());
    }

    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", mime_type, "-i"])
        .stdin(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes)?;
    }
    child.wait()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_manager_loop_prevention() {
        let mgr = ClipboardManager::new();
        let hash = compute_content_hash(b"test-content");

        // Self origin echo should be rejected
        assert!(mgr.is_echo_loop(mgr.origin_id(), &hash));

        // Unknown remote origin with unrecorded hash should NOT be rejected
        assert!(!mgr.is_echo_loop("phone:pixel8", &hash));

        // After recording hash, remote echo should be rejected
        mgr.record_synced_hash(hash);
        assert!(mgr.is_echo_loop("phone:pixel8", &hash));

        // Different hash from remote origin should NOT be rejected
        let other_hash = compute_content_hash(b"other-content");
        assert!(!mgr.is_echo_loop("phone:pixel8", &other_hash));
    }

    #[test]
    fn test_outgoing_message_creation() {
        let mgr = ClipboardManager::new();
        let item = LocalClipboardItem::Text("Hello world".to_string());
        let msg = mgr.create_outgoing_message(item);

        assert_eq!(msg.origin_id, mgr.origin_id());
        assert_eq!(msg.content_type, ClipboardType::Text);
        assert_eq!(msg.seq, 1);

        // Outgoing hash is recorded to prevent echoing when written
        assert!(mgr.is_echo_loop("phone:remote", &msg.content_hash));
    }

    #[test]
    fn test_clipboard_ring_buffer_and_timestamp_ordering() {
        let mgr = ClipboardManager::new();

        // 1. Ring buffer holds up to 32 hashes
        for i in 0..40 {
            let h = compute_content_hash(format!("content-{}", i).as_bytes());
            mgr.record_synced_hash(h);
        }
        // Last one should definitely be present
        let last_h = compute_content_hash(b"content-39");
        assert!(mgr.is_echo_loop("phone:remote", &last_h));
        // Evicted oldest one (0) should no longer be present
        let oldest_h = compute_content_hash(b"content-0");
        assert!(!mgr.is_echo_loop("phone:remote", &oldest_h));

        // 2. Timestamp ordering prevents stale overwrites
        let local_item = LocalClipboardItem::Text("Newer Local Clip".to_string());
        let _ = mgr.create_outgoing_message(local_item);

        // Remote message with older timestamp should be ignored safely
        let older_msg = ClipboardMessage::new_text(
            "phone:remote".to_string(),
            "Older Remote Clip",
            1,
            100, // Very old timestamp
        );
        let res = mgr.apply_remote_clip(&older_msg);
        assert!(res.is_ok());
    }
}
