//! Reassembles video frames from datagram fragments, for low-latency mirroring.
//!
//! The phone splits each encoded frame into ~1 KB datagrams. Datagrams can be
//! lost or arrive slightly out of order, so this keeps a few frames in flight
//! at once instead of abandoning a frame as soon as the next one starts.
//!
//! Frames are delivered strictly in order. A lost frame breaks every frame that
//! references it until the next keyframe, so after a loss the assembler:
//! - asks for a keyframe (the caller sends the request to the phone), and
//! - holds back the broken frames in between, so the screen briefly keeps its
//!   last good image instead of showing corrupted video.

use hyperlink_protocol::video::VideoFrameHeader;

/// Frames that may be in flight at once.
const MAX_PENDING: usize = 4;

/// A complete encoded frame, ready for the decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub data: Vec<u8>,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub width: u16,
    pub height: u16,
}

/// What a fragment led to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// A frame to decode now.
    pub frame: Option<Frame>,
    /// A frame was lost: ask the phone for a keyframe.
    pub request_keyframe: bool,
}

struct Partial {
    id: u32,
    fragments: Vec<Option<Vec<u8>>>,
    received: usize,
    timestamp_us: u64,
    is_keyframe: bool,
    width: u16,
    height: u16,
}

/// `a` is newer than `b`, allowing for wrap-around.
fn newer(a: u32, b: u32) -> bool {
    let diff = a.wrapping_sub(b);
    diff != 0 && diff < u32::MAX / 2
}

#[derive(Default)]
pub struct FrameAssembler {
    pending: Vec<Partial>,
    last_delivered: Option<u32>,
    /// After a loss, drop delta frames until a keyframe arrives.
    waiting_for_keyframe: bool,
    pub frames_ok: u32,
    pub frames_lost: u32,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all state; used when the phone starts a new stream (its frame
    /// numbering restarts).
    pub fn reset(&mut self) {
        *self = Self {
            frames_ok: self.frames_ok,
            frames_lost: self.frames_lost,
            ..Self::default()
        };
    }

    pub fn on_fragment(&mut self, header: &VideoFrameHeader, data: &[u8]) -> Output {
        let mut out = Output::default();
        let id = header.frame_id;

        // Already delivered or skipped past this frame.
        if let Some(last) = self.last_delivered {
            if !newer(id, last) {
                return out;
            }
        }

        let slot = match self.pending.iter().position(|p| p.id == id) {
            Some(i) => i,
            None => {
                if self.pending.len() >= MAX_PENDING {
                    // Too many incomplete frames: the oldest is not coming.
                    let oldest = (0..self.pending.len())
                        .min_by(|&x, &y| {
                            if newer(self.pending[x].id, self.pending[y].id) {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Less
                            }
                        })
                        .unwrap();
                    self.pending.remove(oldest);
                    self.note_loss(&mut out);
                }
                self.pending.push(Partial {
                    id,
                    fragments: vec![None; (header.fragment_count as usize).max(1)],
                    received: 0,
                    timestamp_us: header.timestamp_us,
                    is_keyframe: header.is_keyframe,
                    width: header.width,
                    height: header.height,
                });
                self.pending.len() - 1
            }
        };

        let partial = &mut self.pending[slot];
        let idx = header.fragment_idx as usize;
        if idx >= partial.fragments.len() || partial.fragments[idx].is_some() {
            return out;
        }
        partial.fragments[idx] = Some(data.to_vec());
        partial.received += 1;
        if partial.received < partial.fragments.len() {
            return out;
        }

        // Complete. Anything older still pending can no longer be delivered in
        // order, and a gap in frame numbers means a frame never arrived at all.
        let done = self.pending.remove(slot);
        let before = self.pending.len();
        self.pending.retain(|p| newer(p.id, done.id));
        for _ in 0..(before - self.pending.len()) {
            self.note_loss(&mut out);
        }
        match self.last_delivered {
            Some(last) => {
                if done.id != last.wrapping_add(1) && !self.waiting_for_keyframe {
                    self.note_loss(&mut out);
                }
            }
            // Joined mid-stream: nothing decodes until a keyframe.
            None if !done.is_keyframe => self.note_loss(&mut out),
            None => {}
        }
        self.last_delivered = Some(done.id);

        if done.is_keyframe {
            self.waiting_for_keyframe = false;
        } else if self.waiting_for_keyframe {
            // References a lost frame: showing it would show corruption.
            return out;
        }

        self.frames_ok += 1;
        let mut data = Vec::with_capacity(done.fragments.iter().flatten().map(Vec::len).sum());
        for f in done.fragments.into_iter().flatten() {
            data.extend_from_slice(&f);
        }
        out.frame = Some(Frame {
            data,
            timestamp_us: done.timestamp_us,
            is_keyframe: done.is_keyframe,
            width: done.width,
            height: done.height,
        });
        out
    }

    fn note_loss(&mut self, out: &mut Output) {
        self.frames_lost += 1;
        self.waiting_for_keyframe = true;
        out.request_keyframe = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(id: u32, idx: u16, count: u16, key: bool) -> VideoFrameHeader {
        VideoFrameHeader {
            frame_id: id,
            timestamp_us: id as u64 * 16_000,
            is_keyframe: key,
            payload_len: 1,
            fragment_idx: idx,
            fragment_count: count,
            width: 1080,
            height: 2400,
        }
    }

    #[test]
    fn delivers_in_order_frames() {
        let mut a = FrameAssembler::new();
        assert!(a.on_fragment(&hdr(0, 0, 2, true), b"a").frame.is_none());
        let out = a.on_fragment(&hdr(0, 1, 2, true), b"b");
        assert_eq!(out.frame.unwrap().data, b"ab");
        assert!(!out.request_keyframe);
        assert!(a.on_fragment(&hdr(1, 0, 1, false), b"c").frame.is_some());
        assert_eq!(a.frames_lost, 0);
    }

    #[test]
    fn tolerates_reordering_across_frames() {
        let mut a = FrameAssembler::new();
        a.on_fragment(&hdr(0, 0, 1, true), b"k");
        // Frame 2's first fragment arrives before frame 1's last one.
        a.on_fragment(&hdr(1, 0, 2, false), b"1a");
        a.on_fragment(&hdr(2, 0, 2, false), b"2a");
        let one = a.on_fragment(&hdr(1, 1, 2, false), b"1b");
        assert_eq!(one.frame.unwrap().data, b"1a1b");
        let two = a.on_fragment(&hdr(2, 1, 2, false), b"2b");
        assert_eq!(two.frame.unwrap().data, b"2a2b");
        assert_eq!(a.frames_lost, 0);
    }

    #[test]
    fn loss_requests_keyframe_and_holds_until_one_arrives() {
        let mut a = FrameAssembler::new();
        a.on_fragment(&hdr(0, 0, 1, true), b"k");
        // Frame 1 never arrives; frame 2 completes.
        let out = a.on_fragment(&hdr(2, 0, 1, false), b"p2");
        assert!(out.request_keyframe);
        assert!(
            out.frame.is_none(),
            "a frame after a loss would show corruption"
        );
        assert!(a.on_fragment(&hdr(3, 0, 1, false), b"p3").frame.is_none());
        // The requested keyframe resumes the stream.
        let key = a.on_fragment(&hdr(4, 0, 1, true), b"k4");
        assert_eq!(key.frame.unwrap().data, b"k4");
        assert!(a.on_fragment(&hdr(5, 0, 1, false), b"p5").frame.is_some());
    }

    #[test]
    fn ignores_stragglers_from_delivered_frames() {
        let mut a = FrameAssembler::new();
        a.on_fragment(&hdr(0, 0, 1, true), b"k");
        a.on_fragment(&hdr(1, 0, 1, false), b"p");
        assert_eq!(
            a.on_fragment(&hdr(0, 0, 1, true), b"dup"),
            Output::default()
        );
    }

    #[test]
    fn joining_mid_stream_waits_for_a_keyframe() {
        let mut a = FrameAssembler::new();
        let out = a.on_fragment(&hdr(7, 0, 1, false), b"p");
        assert!(out.request_keyframe);
        assert!(out.frame.is_none());
        assert!(a.on_fragment(&hdr(8, 0, 1, true), b"k").frame.is_some());
    }

    #[test]
    fn reset_accepts_restarted_numbering() {
        let mut a = FrameAssembler::new();
        a.on_fragment(&hdr(0, 0, 1, true), b"k");
        a.on_fragment(&hdr(1, 0, 1, false), b"p");
        a.reset();
        assert!(a.on_fragment(&hdr(0, 0, 1, true), b"k2").frame.is_some());
    }
}
