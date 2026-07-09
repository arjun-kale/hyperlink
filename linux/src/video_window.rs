#![cfg(feature = "video")]

//! Libadwaita video display window for Phase 2 screen mirroring.
//!
//! Creates a GTK4 application window with a `Picture` widget that renders
//! the decoded video from the GStreamer pipeline's paintable sink.

use gtk4::prelude::*;
use gtk4::{self, gdk, Application, EventControllerKey, Picture};
use libadwaita::prelude::*;
use libadwaita::{self as adw, ApplicationWindow, HeaderBar};
use tracing::info;

/// Creates and displays the video mirroring window.
///
/// `paintable` is obtained from `VideoPipeline::paintable()`.
/// Returns the window handle for lifecycle management.
pub fn create_video_window(app: &Application, paintable: &gdk::Paintable) -> ApplicationWindow {
    // Initialize libadwaita.
    adw::init().expect("failed to initialize libadwaita");

    // Build the picture widget displaying the video paintable.
    let picture = Picture::builder()
        .paintable(paintable)
        .hexpand(true)
        .vexpand(true)
        .content_fit(gtk4::ContentFit::Contain)
        .build();

    // Build the content area with header bar.
    let header = HeaderBar::builder()
        .title_widget(&gtk4::Label::new(Some("HyperLink — Screen Mirror")))
        .build();

    // Stats overlay label (FPS, latency, bitrate — updated externally).
    let stats_label = gtk4::Label::builder()
        .label("Waiting for video stream...")
        .css_classes(["caption", "dim-label"])
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::End)
        .margin_end(12)
        .margin_bottom(12)
        .build();

    let overlay = gtk4::Overlay::builder().child(&picture).build();
    overlay.add_overlay(&stats_label);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&overlay);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("HyperLink — Screen Mirror")
        .default_width(960)
        .default_height(540)
        .content(&content)
        .build();

    // Fullscreen toggle: F11
    let win_clone = window.clone();
    let key_controller = EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::F11 {
            if win_clone.is_fullscreen() {
                win_clone.unfullscreen();
            } else {
                win_clone.fullscreen();
            }
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    // Apply dark theme via Adwaita style manager.
    let style_manager = adw::StyleManager::default();
    style_manager.set_color_scheme(adw::ColorScheme::ForceDark);

    window.present();
    info!("video window created and presented");

    window
}

/// Update the stats overlay label with current metrics.
pub fn update_stats_label(
    window: &ApplicationWindow,
    fps: f64,
    bitrate_kbps: u32,
    latency_ms: f64,
) {
    // Find the overlay's stats label by walking the widget tree.
    let content = window.content().expect("window has no content");
    if let Some(vbox) = content.downcast_ref::<gtk4::Box>() {
        if let Some(overlay) = vbox.last_child() {
            if let Some(overlay) = overlay.downcast_ref::<gtk4::Overlay>() {
                // The stats label is the first overlay child.
                let mut child = overlay.first_child();
                while let Some(widget) = child {
                    if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
                        if label.css_classes().iter().any(|c| c == "caption") {
                            label.set_label(&format!(
                                "{:.1} fps | {} kbps | {:.1} ms",
                                fps, bitrate_kbps, latency_ms
                            ));
                            return;
                        }
                    }
                    child = widget.next_sibling();
                }
            }
        }
    }
}
