#![cfg(feature = "video")]

//! Libadwaita video display window for Phase 2 screen mirroring and Phase 3 input capture.
//!
//! Creates a GTK4 application window with a `Picture` widget that renders
//! the decoded video from the GStreamer pipeline's paintable sink, and captures
//! pointer, keyboard, scroll, and navigation events normalized for Android injection.

use gtk4::glib::translate::IntoGlib;
use gtk4::{
    self, gdk, Application, EventControllerKey, EventControllerMotion, EventControllerScroll,
    EventControllerScrollFlags, GestureClick, Picture,
};
use libadwaita::prelude::*;
use libadwaita::{self as adw, ApplicationWindow, HeaderBar};
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::info;

use hyperlink_protocol::input::{
    key_modifiers, KeyAction, KeyEvent, NavAction, NavEvent, PointerAction, PointerButton,
    PointerEvent, ScrollEvent,
};

static VIDEO_WIDTH: AtomicU32 = AtomicU32::new(1080);
static VIDEO_HEIGHT: AtomicU32 = AtomicU32::new(1920);
static UPDATING_DND_UI: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_dnd_button_state(btn: &gtk4::ToggleButton, enabled: bool) {
    if btn.is_active() != enabled {
        UPDATING_DND_UI.store(true, Ordering::Relaxed);
        btn.set_active(enabled);
        UPDATING_DND_UI.store(false, Ordering::Relaxed);
    }
}

pub fn set_video_dimensions(width: u32, height: u32) {
    if width > 0 && height > 0 {
        VIDEO_WIDTH.store(width, Ordering::Relaxed);
        VIDEO_HEIGHT.store(height, Ordering::Relaxed);
    }
}

pub fn get_video_dimensions() -> (f64, f64) {
    (
        VIDEO_WIDTH.load(Ordering::Relaxed) as f64,
        VIDEO_HEIGHT.load(Ordering::Relaxed) as f64,
    )
}

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Normalizes mouse (x, y) coordinates on a ContentFit::Contain Picture widget
/// into fixed-point u16 (0..65535) phone coordinates.
///
/// Returns None if the mouse is currently in the letterbox/pillarbox padding.
pub fn normalize_coordinates(
    widget_width: f64,
    widget_height: f64,
    x: f64,
    y: f64,
) -> Option<(u16, u16)> {
    let (video_w, video_h) = get_video_dimensions();
    if widget_width <= 0.0 || widget_height <= 0.0 || video_w <= 0.0 || video_h <= 0.0 {
        return None;
    }

    let scale = (widget_width / video_w).min(widget_height / video_h);
    let render_width = video_w * scale;
    let render_height = video_h * scale;
    let offset_x = (widget_width - render_width) / 2.0;
    let offset_y = (widget_height - render_height) / 2.0;

    if x < offset_x || x > offset_x + render_width || y < offset_y || y > offset_y + render_height {
        return None;
    }

    let norm_x = (x - offset_x) / render_width;
    let norm_y = (y - offset_y) / render_height;

    Some((
        PointerEvent::float_to_norm(norm_x),
        PointerEvent::float_to_norm(norm_y),
    ))
}

/// Creates and displays the video mirroring window.
///
/// `paintable` is obtained from `VideoPipeline::paintable()`.
/// Returns the window handle and toast overlay for notification toasts.
pub fn create_video_window(
    app: &Application,
    paintable: &gdk::Paintable,
    config_path: std::path::PathBuf,
) -> (ApplicationWindow, adw::ToastOverlay, gtk4::ToggleButton) {
    // Initialize libadwaita.
    adw::init().expect("failed to initialize libadwaita");

    // Build the picture widget displaying the video paintable.
    let picture = Picture::builder()
        .paintable(paintable)
        .hexpand(true)
        .vexpand(true)
        .content_fit(gtk4::ContentFit::Contain)
        .build();

    // 1. GestureClick controller on the video picture widget (clicks/taps)
    let gesture_click = GestureClick::new();
    gesture_click.set_button(0); // Listen to all mouse buttons
    let pic_clone1 = picture.clone();
    gesture_click.connect_pressed(move |gesture, _, x, y| {
        let button = match gesture.current_button() {
            1 => PointerButton::Primary,
            2 => PointerButton::Middle,
            3 => PointerButton::Secondary,
            _ => PointerButton::Primary,
        };

        // Right-click maps directly to Android Back button navigation
        if button == PointerButton::Secondary {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::Nav(NavEvent {
                    action: NavAction::Back,
                    timestamp_us: now_us(),
                }));
            }
            return;
        }

        let w = pic_clone1.width() as f64;
        let h = pic_clone1.height() as f64;
        if let Some((x_norm, y_norm)) = normalize_coordinates(w, h, x, y) {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::Pointer(PointerEvent {
                    action: PointerAction::Down,
                    button,
                    x_norm,
                    y_norm,
                    pressure: 128,
                    timestamp_us: now_us(),
                }));
            }
        }
    });

    let pic_clone2 = picture.clone();
    gesture_click.connect_released(move |gesture, _, x, y| {
        let button = match gesture.current_button() {
            1 => PointerButton::Primary,
            2 => PointerButton::Middle,
            3 => PointerButton::Secondary,
            _ => PointerButton::Primary,
        };
        if button == PointerButton::Secondary {
            return;
        }

        let w = pic_clone2.width() as f64;
        let h = pic_clone2.height() as f64;
        if let Some((x_norm, y_norm)) = normalize_coordinates(w, h, x, y) {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::Pointer(PointerEvent {
                    action: PointerAction::Up,
                    button,
                    x_norm,
                    y_norm,
                    pressure: 0,
                    timestamp_us: now_us(),
                }));
            }
        }
    });
    picture.add_controller(gesture_click);

    // 2. Motion controller on picture widget (pointer move / drag)
    let motion_controller = EventControllerMotion::new();
    let pic_clone3 = picture.clone();
    motion_controller.connect_motion(move |_, x, y| {
        let w = pic_clone3.width() as f64;
        let h = pic_clone3.height() as f64;
        if let Some((x_norm, y_norm)) = normalize_coordinates(w, h, x, y) {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::Pointer(PointerEvent {
                    action: PointerAction::Move,
                    button: PointerButton::None,
                    x_norm,
                    y_norm,
                    pressure: 0,
                    timestamp_us: now_us(),
                }));
            }
        }
    });
    picture.add_controller(motion_controller);

    // 3. Scroll controller on picture widget (mouse wheel deltas)
    let scroll_controller = EventControllerScroll::new(EventControllerScrollFlags::BOTH_AXES);
    let pic_clone4 = picture.clone();
    scroll_controller.connect_scroll(move |_, dx, dy| {
        let w = pic_clone4.width() as f64;
        let h = pic_clone4.height() as f64;
        let (x_norm, y_norm) =
            normalize_coordinates(w, h, w / 2.0, h / 2.0).unwrap_or((32768, 32768));
        if let Some(tx) = crate::INPUT_SENDER.get() {
            let _ = tx.try_send(crate::InputGuiMessage::Scroll(ScrollEvent {
                dx: (dx * 120.0) as i16,
                dy: (dy * 120.0) as i16,
                x_norm,
                y_norm,
                timestamp_us: now_us(),
            }));
        }
        gtk4::glib::Propagation::Stop
    });
    picture.add_controller(scroll_controller);

    // Build the content area with header bar.
    let header = HeaderBar::builder()
        .title_widget(&gtk4::Label::new(Some("HyperLink — Screen Mirror")))
        .build();

    // DND toggle button in header
    let dnd_button = gtk4::ToggleButton::builder()
        .tooltip_text("Do-Not-Disturb (Mute Notifications)")
        .icon_name("notifications-disabled-symbolic")
        .build();
    dnd_button.connect_toggled(move |btn| {
        if UPDATING_DND_UI.load(Ordering::Relaxed) {
            return;
        }
        let is_active = btn.is_active();
        crate::set_global_dnd_active(is_active);
        if let Some(tx) = crate::INPUT_SENDER.get() {
            let _ = tx.try_send(crate::InputGuiMessage::Dnd(
                hyperlink_protocol::notification::DndSync {
                    dnd_enabled: is_active,
                },
            ));
        }
    });
    header.pack_end(&dnd_button);

    // Preferences button in header (Phase 11) — click handler wired below once
    // `window` exists, since the preferences window needs a transient-for parent.
    let prefs_button = gtk4::Button::builder()
        .tooltip_text("Preferences")
        .icon_name("preferences-system-symbolic")
        .build();
    header.pack_end(&prefs_button);

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

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&overlay));

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&toast_overlay);

    // Load saved workflow state if present
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let wf_path = std::path::PathBuf::from(home).join(".config/hyperlink/workflow_state.json");
    let saved_wf = if wf_path.exists() {
        std::fs::File::open(&wf_path)
            .ok()
            .and_then(|f| serde_json::from_reader(std::io::BufReader::new(f)).ok())
            .unwrap_or_default()
    } else {
        hyperlink_protocol::proximity::WorkflowState::default()
    };

    let window = ApplicationWindow::builder()
        .application(app)
        .title("HyperLink — Screen Mirror")
        .default_width(saved_wf.window_width as i32)
        .default_height(saved_wf.window_height as i32)
        .content(&content)
        .build();

    if saved_wf.is_fullscreen {
        window.fullscreen();
    }

    let window_for_prefs = window.clone();
    prefs_button.connect_clicked(move |_| {
        crate::preferences_window::show_preferences_window(&window_for_prefs, config_path.clone());
    });

    let win_for_close = window.clone();
    let wf_path_for_close = wf_path.clone();
    window.connect_close_request(move |_| {
        let (w, h) = (win_for_close.width() as u32, win_for_close.height() as u32);
        let is_fs = win_for_close.is_fullscreen();
        let state = hyperlink_protocol::proximity::WorkflowState {
            window_width: w,
            window_height: h,
            window_x: -1,
            window_y: -1,
            is_fullscreen: is_fs,
            active_package_name: "com.android.launcher".to_string(),
            display_orientation: 0,
            saved_timestamp_us: now_us(),
        };
        if let Some(parent) = wf_path_for_close.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(&wf_path_for_close, json);
        }
        gtk4::glib::Propagation::Proceed
    });

    // 4. Key controller on window (F11 fullscreen, Esc -> Back, Super -> Home, alphanumeric keys)
    let win_clone = window.clone();
    let key_controller = EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        if keyval == gdk::Key::F11 {
            if win_clone.is_fullscreen() {
                win_clone.unfullscreen();
            } else {
                win_clone.fullscreen();
            }
            return gtk4::glib::Propagation::Stop;
        }

        if keyval == gdk::Key::Escape {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::Nav(NavEvent {
                    action: NavAction::Back,
                    timestamp_us: now_us(),
                }));
            }
            return gtk4::glib::Propagation::Stop;
        }

        if keyval == gdk::Key::Super_L || keyval == gdk::Key::Super_R {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::Nav(NavEvent {
                    action: NavAction::Home,
                    timestamp_us: now_us(),
                }));
            }
            return gtk4::glib::Propagation::Stop;
        }

        let mut modifiers = 0u8;
        if state.contains(gdk::ModifierType::SHIFT_MASK) {
            modifiers |= key_modifiers::SHIFT;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK) {
            modifiers |= key_modifiers::CTRL;
        }
        if state.contains(gdk::ModifierType::ALT_MASK) {
            modifiers |= key_modifiers::ALT;
        }
        if state.contains(gdk::ModifierType::SUPER_MASK) {
            modifiers |= key_modifiers::META;
        }

        let wire_keycode = keyval
            .to_unicode()
            .map(|c| c as u32)
            .unwrap_or_else(|| keyval.into_glib());

        if let Some(tx) = crate::INPUT_SENDER.get() {
            let _ = tx.try_send(crate::InputGuiMessage::Key(KeyEvent {
                action: KeyAction::Down,
                keycode: wire_keycode,
                modifiers,
                timestamp_us: now_us(),
            }));
        }

        gtk4::glib::Propagation::Proceed
    });

    key_controller.connect_key_released(move |_, keyval, _keycode, state| {
        if keyval == gdk::Key::F11
            || keyval == gdk::Key::Escape
            || keyval == gdk::Key::Super_L
            || keyval == gdk::Key::Super_R
        {
            return;
        }

        let mut modifiers = 0u8;
        if state.contains(gdk::ModifierType::SHIFT_MASK) {
            modifiers |= key_modifiers::SHIFT;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK) {
            modifiers |= key_modifiers::CTRL;
        }
        if state.contains(gdk::ModifierType::ALT_MASK) {
            modifiers |= key_modifiers::ALT;
        }
        if state.contains(gdk::ModifierType::SUPER_MASK) {
            modifiers |= key_modifiers::META;
        }

        let wire_keycode = keyval
            .to_unicode()
            .map(|c| c as u32)
            .unwrap_or_else(|| keyval.into_glib());

        if let Some(tx) = crate::INPUT_SENDER.get() {
            let _ = tx.try_send(crate::InputGuiMessage::Key(KeyEvent {
                action: KeyAction::Up,
                keycode: wire_keycode,
                modifiers,
                timestamp_us: now_us(),
            }));
        }
    });
    window.add_controller(key_controller);

    // Apply dark theme via Adwaita style manager.
    let style_manager = adw::StyleManager::default();
    style_manager.set_color_scheme(adw::ColorScheme::ForceDark);

    window.present();
    info!("video window created and presented");

    (window, toast_overlay, dnd_button)
}

/// Displays an in-app Libadwaita notification toast banner with click-through action.
pub fn show_notification_toast(
    window: &ApplicationWindow,
    toast_overlay: &adw::ToastOverlay,
    notif: hyperlink_protocol::notification::NotificationPost,
) {
    let summary = if notif.app_name.is_empty() {
        format!("{}: {}", notif.title, notif.body)
    } else {
        format!("[{}] {}: {}", notif.app_name, notif.title, notif.body)
    };

    let toast = adw::Toast::new(&summary);
    toast.set_timeout(5); // 5 seconds display

    let (btn_label, action_id_opt) = if let Some(action) = notif.actions.first() {
        (action.title.clone(), Some(action.action_id))
    } else {
        ("Open Mirror".to_string(), None)
    };
    toast.set_button_label(Some(&btn_label));

    let win_clone = window.clone();
    let notif_id_action = notif.id.clone();
    toast.connect_button_clicked(move |_| {
        win_clone.present();
        if let Some(action_id) = action_id_opt {
            if let Some(tx) = crate::INPUT_SENDER.get() {
                let _ = tx.try_send(crate::InputGuiMessage::NotificationAction(
                    hyperlink_protocol::notification::NotificationActionInvoke {
                        id: notif_id_action.clone(),
                        action_id,
                    },
                ));
            }
        }
    });

    let notif_id_dismiss = notif.id.clone();
    toast.connect_dismissed(move |_| {
        if let Some(tx) = crate::INPUT_SENDER.get() {
            let _ = tx.try_send(crate::InputGuiMessage::NotificationDismiss(
                hyperlink_protocol::notification::NotificationDismiss {
                    id: notif_id_dismiss.clone(),
                },
            ));
        }
    });

    toast_overlay.add_toast(toast);
}

/// Update the stats overlay label with current metrics.
///
/// `latency_ms` is `Some(ms)` only when a calibrated cross-device clock-sync offset
/// is available. If `None`, latency is omitted to avoid displaying misleading numbers.
pub fn update_stats_label(
    window: &ApplicationWindow,
    fps: f64,
    bitrate_kbps: u32,
    latency_ms: Option<f64>,
) {
    // Find the overlay's stats label by walking the widget tree.
    let content = window.content().expect("window has no content");
    if let Some(vbox) = content.downcast_ref::<gtk4::Box>() {
        if let Some(last_child) = vbox.last_child() {
            let overlay_opt = if let Some(to) = last_child.downcast_ref::<adw::ToastOverlay>() {
                to.child().and_then(|c| c.downcast::<gtk4::Overlay>().ok())
            } else {
                last_child.downcast_ref::<gtk4::Overlay>().cloned()
            };

            if let Some(overlay) = overlay_opt {
                // The stats label is the first overlay child.
                let mut child = overlay.first_child();
                while let Some(widget) = child {
                    if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
                        if label.css_classes().iter().any(|c| c == "caption") {
                            let text = match latency_ms {
                                Some(lat) if lat > 0.0 => format!(
                                    "{:.1} fps | {} kbps | {:.1} ms",
                                    fps, bitrate_kbps, lat
                                ),
                                _ => format!("{:.1} fps | {} kbps", fps, bitrate_kbps),
                            };
                            label.set_label(&text);
                            return;
                        }
                    }
                    child = widget.next_sibling();
                }
            }
        }
    }
}
