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
/// Pipeline layout:
///   appsrc → h264parse → (vaapidecodebin | avdec_h264) → videoconvert → gtk4paintablesink
pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    frame_count: u64,
    base_pts: Option<u64>,
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

        // Sink: gtk4paintablesink for GTK4 integration.
        let sink = gst::ElementFactory::make("gtk4paintablesink")
            .name("video-sink")
            .build()
            .context(
                "failed to create gtk4paintablesink — install gstreamer1.0-plugins-bad with GTK4 support",
            )?;

        // Add all elements and link.
        pipeline
            .add_many([appsrc.upcast_ref(), &parser, &decoder, &convert, &sink])
            .context("failed to add elements to pipeline")?;

        gst::Element::link_many([appsrc.upcast_ref(), &parser, &decoder, &convert, &sink])
            .context("failed to link pipeline elements")?;

        // Set up error handling on the bus.
        let bus = pipeline.bus().unwrap();
        bus.add_watch(move |_, msg| {
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
            gst::BusSyncReply::Pass.into()
        })
        .context("failed to add bus watch")?;

        Ok(Self {
            pipeline,
            appsrc,
            frame_count: 0,
            base_pts: None,
        })
    }

    /// Try VAAPI hardware decoder first, fall back to software on failure.
    fn create_decoder_with_fallback() -> Result<gst::Element> {
        // Try vaapidecodebin (Intel/AMD).
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
        gst::ElementFactory::make("avdec_h264")
            .name("decoder")
            .build()
            .context("failed to create avdec_h264 software decoder")
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

    /// Push a raw H.264 NAL unit into the pipeline for decoding.
    ///
    /// `timestamp_us` is the capture timestamp from the sender.
    /// `is_keyframe` marks IDR frames for GStreamer buffer flags.
    pub fn push_frame(&mut self, nal_data: &[u8], timestamp_us: u64, is_keyframe: bool) {
        // Set base PTS from the first frame's timestamp.
        let base = *self.base_pts.get_or_insert(timestamp_us);
        let pts_ns = (timestamp_us.saturating_sub(base)) * 1000; // µs → ns

        let mut buffer = gst::Buffer::with_size(nal_data.len()).unwrap();
        {
            let buffer_ref = buffer.get_mut().unwrap();
            buffer_ref.set_pts(ClockTime::from_nseconds(pts_ns));

            if is_keyframe {
                // No special flags needed; h264parse infers keyframes from NAL type.
            } else {
                buffer_ref.set_flags(gst::BufferFlags::DELTA_UNIT);
            }

            let mut map = buffer_ref.map_writable().unwrap();
            map.copy_from_slice(nal_data);
        }

        if let Err(e) = self.appsrc.push_buffer(buffer) {
            error!("failed to push buffer to appsrc: {}", e);
        }

        self.frame_count += 1;
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

    /// Returns the total number of frames pushed so far.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
