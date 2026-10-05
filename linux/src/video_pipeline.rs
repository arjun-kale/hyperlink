#![cfg(feature = "video")]

//! GStreamer video decode pipeline for Phase 2 screen mirroring.
//!
//! Receives H.264 NAL units from the QUIC video stream and decodes them
//! into a GTK4-paintable surface for display. Uses hardware decode (VAAPI)
//! when available, falling back to software decode (`avdec_h264`).

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer::{self as gst, ClockTime};
use gstreamer_app as gst_app;
use tracing::{error, info, warn};

/// Encapsulates the GStreamer decode pipeline.
///
/// Pipeline layout (tuned for latency, not smoothness):
///   appsrc → h264parse → (vah264dec | vaapidecodebin | nvh264dec | avdec_h264)
///     → videoconvert → queue (1 frame, leaky) → gtk4paintablesink (sync=false)
///
/// The sink shows each frame as soon as it's decoded instead of waiting for its
/// timestamp, and the leaky one-frame queue drops a decoded frame rather than
/// let them pile up behind a busy UI thread: with live mirroring, an old frame
/// is never worth showing.
pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    _bus_watch_guard: gst::bus::BusWatchGuard,
}

/// A thread-safe handle for feeding encoded frames into the pipeline, used
/// directly from the network task so video never waits on the UI thread.
#[derive(Clone)]
pub struct VideoInput {
    appsrc: gst_app::AppSrc,
    base_pts: std::sync::Arc<std::sync::Mutex<Option<u64>>>,
}

impl VideoPipeline {
    /// Create a new pipeline. The pipeline is initially in the NULL state.
    ///
    /// `use_hardware` controls whether to attempt VAAPI hardware decode.
    /// If `true` and VAAPI is unavailable, automatically falls back to software.
    pub fn new(use_hardware: bool) -> Result<Self> {
        gst::init().context("failed to initialize GStreamer")?;

        let pipeline = gst::Pipeline::builder()
            .name("hyperlink-video-pipeline")
            .build();

        // Source: appsrc receives pushed NAL buffers from the QUIC layer.
        let appsrc = gst_app::AppSrc::builder()
            .name("video-src")
            .is_live(true)
            .format(gst::Format::Time)
            .caps(
                &gst::Caps::builder("video/x-h264")
                    .field("stream-format", "byte-stream")
                    .field("alignment", "au")
                    .build(),
            )
            .build();

        // Parser: h264parse to clean up NAL framing.
        let parser = gst::ElementFactory::make("h264parse")
            .name("parser")
            .build()
            .context("failed to create h264parse element")?;

        // Decoder: try hardware first, then software.
        let decoder = if use_hardware {
            Self::create_decoder_with_fallback()?
        } else {
            Self::create_software_decoder()?
        };

        // Color converter for format compatibility.
        let convert = gst::ElementFactory::make("videoconvert")
            .name("convert")
            .build()
            .context("failed to create videoconvert element")?;

        // One decoded frame of slack, dropping the older one when full. Also
        // decouples the decoder thread from the sink, which hands frames to the
        // GTK main thread.
        let queue = gst::ElementFactory::make("queue")
            .name("latest-frame")
            .property("max-size-buffers", 1u32)
            .property("max-size-bytes", 0u32)
            .property("max-size-time", 0u64)
            .build()
            .context("failed to create queue element")?;
        queue.set_property_from_str("leaky", "downstream");

        // Sink: gtk4paintablesink for GTK4 integration. sync=false: show frames
        // as they arrive; there's nothing to synchronize a live mirror against.
        let sink = gst::ElementFactory::make("gtk4paintablesink")
            .name("video-sink")
            .property("sync", false)
            .build()
            .context(
                "failed to create gtk4paintablesink — install gstreamer1.0-plugins-bad with GTK4 support",
            )?;

        // Add all elements and link.
        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &parser,
                &decoder,
                &convert,
                &queue,
                &sink,
            ])
            .context("failed to add elements to pipeline")?;

        gst::Element::link_many([
            appsrc.upcast_ref(),
            &parser,
            &decoder,
            &convert,
            &queue,
            &sink,
        ])
        .context("failed to link pipeline elements")?;

        // Set up error handling on the bus.
        let bus = pipeline.bus().unwrap();
        let bus_watch_guard = bus
            .add_watch(move |_, msg| {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Error(err) => {
                        error!(
                            "GStreamer error from {:?}: {} ({:?})",
                            err.src().map(|s| s.path_string()),
                            err.error(),
                            err.debug()
                        );
                    }
                    MessageView::Warning(warn) => {
                        warn!(
                            "GStreamer warning from {:?}: {}",
                            warn.src().map(|s| s.path_string()),
                            warn.error()
                        );
                    }
                    MessageView::Eos(_) => {
                        info!("GStreamer pipeline reached end-of-stream");
                    }
                    _ => {}
                }
                gst::glib::ControlFlow::Continue
            })
            .context("failed to add bus watch")?;

        Ok(Self {
            pipeline,
            appsrc,
            _bus_watch_guard: bus_watch_guard,
        })
    }

    /// Try hardware decoders first, fall back to software on failure.
    fn create_decoder_with_fallback() -> Result<gst::Element> {
        // GStreamer's current VA-API plugin (Intel/AMD); the older vaapi
        // elements below are deprecated and often not installed anymore.
        if let Ok(dec) = gst::ElementFactory::make("vah264dec")
            .name("decoder")
            .build()
        {
            info!("using VA hardware decoder (vah264dec)");
            return Ok(dec);
        }

        // Try vaapidecodebin (Intel/AMD, legacy plugin).
        if let Ok(dec) = gst::ElementFactory::make("vaapidecodebin")
            .name("decoder")
            .build()
        {
            info!("using VAAPI hardware decoder");
            return Ok(dec);
        }

        // Try NVDEC (NVIDIA).
        if let Ok(dec) = gst::ElementFactory::make("nvh264dec")
            .name("decoder")
            .build()
        {
            info!("using NVDEC hardware decoder");
            return Ok(dec);
        }

        warn!("no hardware decoder available, falling back to software decode");
        Self::create_software_decoder()
    }

    /// Create software-only H.264 decoder.
    fn create_software_decoder() -> Result<gst::Element> {
        let dec = gst::ElementFactory::make("avdec_h264")
            .name("decoder")
            .build()
            .context("failed to create avdec_h264 software decoder")?;
        // Frame threading holds (threads - 1) frames before output; slice
        // threading adds no delay.
        if dec.find_property("thread-type").is_some() {
            dec.set_property_from_str("thread-type", "slice");
        }
        Ok(dec)
    }

    /// A handle for pushing frames from any thread.
    pub fn input(&self) -> VideoInput {
        VideoInput {
            appsrc: self.appsrc.clone(),
            base_pts: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Returns the GDK paintable from the sink, for binding to a GTK4 `Picture` widget.
    ///
    /// Must be called after `new()` but before starting the pipeline.
    pub fn paintable(&self) -> Result<gtk4::gdk::Paintable> {
        let sink = self
            .pipeline
            .by_name("video-sink")
            .context("video-sink element not found")?;
        let paintable = sink.property::<gtk4::gdk::Paintable>("paintable");
        Ok(paintable)
    }

    /// Start the pipeline (transition to PLAYING state).
    pub fn start(&self) -> Result<()> {
        self.pipeline
            .set_state(gst::State::Playing)
            .context("failed to set pipeline to PLAYING")?;
        info!("video pipeline started");
        Ok(())
    }

    /// Stop the pipeline (transition to NULL state).
    pub fn stop(&self) -> Result<()> {
        self.pipeline
            .set_state(gst::State::Null)
            .context("failed to set pipeline to NULL")?;
        info!("video pipeline stopped");
        Ok(())
    }

    /// Push SPS/PPS codec data as a stream header.
    ///
    /// This is sent once before the first frame and again on each keyframe
    /// to allow the decoder to (re)initialize.
    pub fn set_codec_data(&self, sps: &[u8], pps: &[u8]) {
        // Build an AnnexB byte-stream with start codes: [0,0,0,1,SPS,0,0,0,1,PPS]
        let mut codec_data = Vec::with_capacity(4 + sps.len() + 4 + pps.len());
        codec_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        codec_data.extend_from_slice(sps);
        codec_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        codec_data.extend_from_slice(pps);

        let mut buffer = gst::Buffer::with_size(codec_data.len()).unwrap();
        {
            let buffer_ref = buffer.get_mut().unwrap();
            buffer_ref.set_flags(gst::BufferFlags::HEADER);
            let mut map = buffer_ref.map_writable().unwrap();
            map.copy_from_slice(&codec_data);
        }

        if let Err(e) = self.appsrc.push_buffer(buffer) {
            error!("failed to push codec data to appsrc: {}", e);
        }

        info!(
            "pushed codec config: SPS={} bytes, PPS={} bytes",
            sps.len(),
            pps.len()
        );
    }
}

impl VideoInput {
    /// Push one encoded H.264 access unit. Safe to call from any thread.
    ///
    /// `timestamp_us` is the sender's capture timestamp; it's only used to keep
    /// buffer timestamps increasing, since the sink doesn't sync to them.
    pub fn push_frame(&self, nal_data: &[u8], timestamp_us: u64, is_keyframe: bool) {
        let base = *self.base_pts.lock().unwrap().get_or_insert(timestamp_us);
        let pts_ns = timestamp_us.saturating_sub(base) * 1000; // µs → ns

        let mut buffer = gst::Buffer::from_mut_slice(nal_data.to_vec());
        {
            let buffer_ref = buffer.get_mut().unwrap();
            buffer_ref.set_pts(ClockTime::from_nseconds(pts_ns));
            if !is_keyframe {
                buffer_ref.set_flags(gst::BufferFlags::DELTA_UNIT);
            }
        }

        if let Err(e) = self.appsrc.push_buffer(buffer) {
            error!("failed to push buffer to appsrc: {}", e);
        }
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
