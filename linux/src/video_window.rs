#![cfg(feature = "video")]

//! The mirrored phone screen (Phase 2) and input capture (Phase 3).
//!
//! Builds the mirror view shown inside the main window: the phone's screen on
//! a dim stage, a floating glass dock with the phone's navigation keys, and
//! the pointer/scroll/keyboard controllers that turn desktop input into
//! normalized Android input events.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use gtk4::{
    gdk, EventControllerKey, EventControllerMotion, EventControllerScroll,
    EventControllerScrollFlags, GestureClick, Picture,
};
use libadwaita as adw;

use crate::glass::{AmbientStage, GlassPanel};
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

/// Records the phone's current frame size. Returns true if it changed.
pub fn set_video_dimensions(width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let old_w = VIDEO_WIDTH.swap(width, Ordering::Relaxed);
    let old_h = VIDEO_HEIGHT.swap(height, Ordering::Relaxed);
    old_w != width || old_h != height
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

fn send(msg: crate::InputGuiMessage) {
    if let Some(tx) = crate::INPUT_SENDER.get() {
        let _ = tx.try_send(msg);
    }
}

fn send_nav(action: NavAction) {
    send(crate::InputGuiMessage::Nav(NavEvent {
        action,
        timestamp_us: now_us(),
    }));
}

/// The mirror view and the handles the app updates while a stream is live.
pub struct MirrorView {
    pub root: gtk4::Widget,
    pub frame: gtk4::AspectFrame,
    pub stats_panel: GlassPanel,
    pub stats_label: gtk4::Label,
    pub paused_panel: GlassPanel,
    pub dnd_button: gtk4::ToggleButton,
}

impl MirrorView {
    /// Matches the phone frame to the stream's aspect ratio, so the rounded
    /// corners sit on the video rather than on letterbox padding.
    pub fn set_aspect(&self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.frame.set_ratio(width as f32 / height as f32);
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused_panel.set_visible(paused);
    }

    pub fn set_stats(&self, fps: f64, bitrate_kbps: u32) {
        self.stats_label.set_label(&format!(
            "{fps:.0} fps · {:.1} Mbps",
            bitrate_kbps as f64 / 1000.0
        ));
    }
}

fn dock_button(icon: &str, tooltip: &str, action: NavAction) -> gtk4::Button {
    let button = gtk4::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["hl-dock-button"])
        .build();
    button.update_property(&[gtk4::accessible::Property::Label(tooltip)]);
    button.connect_clicked(move |_| send_nav(action));
    button
}

fn dock_separator() -> gtk4::Widget {
    gtk4::Box::builder()
        .css_classes(["hl-dock-separator"])
        .build()
        .upcast()
}

/// Builds the mirror view around the decoded video `paintable`.
pub fn build_mirror_view(paintable: &gdk::Paintable) -> MirrorView {
    let picture = Picture::builder()
        .paintable(paintable)
        .hexpand(true)
        .vexpand(true)
        .content_fit(gtk4::ContentFit::Contain)
        .can_shrink(true)
        .css_classes(["hl-phone"])
        .overflow(gtk4::Overflow::Hidden)
        .build();
    picture.update_property(&[gtk4::accessible::Property::Label(
        "Your phone's screen. Click to tap, scroll to swipe, type to enter text.",
    )]);
    attach_pointer_controllers(&picture);

    let (w, h) = get_video_dimensions();
    let frame = gtk4::AspectFrame::builder()
        .ratio((w / h) as f32)
        .obey_child(false)
        .child(&picture)
        .margin_top(24)
        .margin_bottom(104) // room for the dock, so it never covers the screen's bottom edge
        .margin_start(24)
        .margin_end(24)
        .build();

    // ── Dock ──
    let dnd_button = gtk4::ToggleButton::builder()
        .icon_name("notifications-disabled-symbolic")
        .tooltip_text("Do Not Disturb on phone and computer")
        .css_classes(["hl-dock-button"])
        .build();
    dnd_button.update_property(&[gtk4::accessible::Property::Label("Do Not Disturb")]);
    dnd_button.connect_toggled(|btn| {
        if UPDATING_DND_UI.load(Ordering::Relaxed) {
            return;
        }
        let active = btn.is_active();
        crate::set_global_dnd_active(active);
        send(crate::InputGuiMessage::Dnd(
            hyperlink_protocol::notification::DndSync {
                dnd_enabled: active,
            },
        ));
    });

    let dock_row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(2)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    dock_row.append(&dock_button(
        "go-previous-symbolic",
        "Back  (Esc or right-click)",
        NavAction::Back,
    ));
    dock_row.append(&dock_button(
        "go-home-symbolic",
        "Home  (Super)",
        NavAction::Home,
    ));
    dock_row.append(&dock_button(
        "view-app-grid-symbolic",
        "Recent apps",
        NavAction::Recents,
    ));
    dock_row.append(&dock_separator());
    dock_row.append(&dock_button(
        "audio-volume-low-symbolic",
        "Volume down",
        NavAction::VolumeDown,
    ));
    dock_row.append(&dock_button(
        "audio-volume-high-symbolic",
        "Volume up",
        NavAction::VolumeUp,
    ));
    dock_row.append(&dock_separator());
    dock_row.append(&dnd_button);

    let overlay = gtk4::Overlay::builder().child(&frame).build();
    let stage = AmbientStage::new(&picture, &overlay);

    let dock = GlassPanel::new(&picture, &stage, &dock_row, 22.0);
    dock.set_halign(gtk4::Align::Center);
    dock.set_valign(gtk4::Align::End);
    dock.set_margin_bottom(28);

    // ── Stream stats (hidden until asked for) ──
    let stats_label = gtk4::Label::builder()
        .label("Waiting for video…")
        .css_classes(["caption", "hl-stats"])
        .build();
    let stats_panel = GlassPanel::new(&picture, &stage, &stats_label, 14.0);
    stats_panel.set_halign(gtk4::Align::End);
    stats_panel.set_valign(gtk4::Align::Start);
    stats_panel.set_margin_top(16);
    stats_panel.set_margin_end(16);
    stats_panel.set_visible(false);

    // ── Paused state ──
    let paused_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(28)
        .margin_end(28)
        .build();
    paused_box.append(
        &gtk4::Image::builder()
            .icon_name("media-playback-pause-symbolic")
            .pixel_size(32)
            .margin_bottom(6)
            .build(),
    );
    paused_box.append(
        &gtk4::Label::builder()
            .label("Screen sharing paused")
            .css_classes(["heading"])
            .build(),
    );
    paused_box.append(
        &gtk4::Label::builder()
            .label("Your phone stopped sending its screen.\nTap “Show screen on PC” on your phone to resume.")
            .justify(gtk4::Justification::Center)
            .build(),
    );
    let paused_panel = GlassPanel::new(&picture, &stage, &paused_box, 24.0);
    paused_panel.set_halign(gtk4::Align::Center);
    paused_panel.set_valign(gtk4::Align::Center);
    paused_panel.set_visible(false);

    overlay.add_overlay(&dock);
    overlay.add_overlay(&stats_panel);
    overlay.add_overlay(&paused_panel);

    MirrorView {
        root: stage.upcast(),
        frame,
        stats_panel,
        stats_label,
        paused_panel,
        dnd_button,
    }
}

fn pointer_button(gesture: &GestureClick) -> PointerButton {
    match gesture.current_button() {
        2 => PointerButton::Middle,
        3 => PointerButton::Secondary,
        _ => PointerButton::Primary,
    }
}

fn attach_pointer_controllers(picture: &Picture) {
    // Clicks become taps; right-click is Android's Back.
    let gesture_click = GestureClick::new();
    gesture_click.set_button(0);
    let pic = picture.clone();
    gesture_click.connect_pressed(move |gesture, _, x, y| {
        let button = pointer_button(gesture);
        if button == PointerButton::Secondary {
            send_nav(NavAction::Back);
            return;
        }
        if let Some((x_norm, y_norm)) =
            normalize_coordinates(pic.width() as f64, pic.height() as f64, x, y)
        {
            send(crate::InputGuiMessage::Pointer(PointerEvent {
                action: PointerAction::Down,
                button,
                x_norm,
                y_norm,
                pressure: 128,
                timestamp_us: now_us(),
            }));
        }
    });
    let pic = picture.clone();
    gesture_click.connect_released(move |gesture, _, x, y| {
        let button = pointer_button(gesture);
        if button == PointerButton::Secondary {
            return;
        }
        if let Some((x_norm, y_norm)) =
            normalize_coordinates(pic.width() as f64, pic.height() as f64, x, y)
        {
            send(crate::InputGuiMessage::Pointer(PointerEvent {
                action: PointerAction::Up,
                button,
                x_norm,
                y_norm,
                pressure: 0,
                timestamp_us: now_us(),
            }));
        }
    });
    picture.add_controller(gesture_click);

    // Pointer motion (drags).
    let motion_controller = EventControllerMotion::new();
    let pic = picture.clone();
    motion_controller.connect_motion(move |_, x, y| {
        if let Some((x_norm, y_norm)) =
            normalize_coordinates(pic.width() as f64, pic.height() as f64, x, y)
        {
            send(crate::InputGuiMessage::Pointer(PointerEvent {
                action: PointerAction::Move,
                button: PointerButton::None,
                x_norm,
                y_norm,
                pressure: 0,
                timestamp_us: now_us(),
            }));
        }
    });
    picture.add_controller(motion_controller);

    // Mouse wheel scrolls the phone.
    let scroll_controller = EventControllerScroll::new(EventControllerScrollFlags::BOTH_AXES);
    let pic = picture.clone();
    scroll_controller.connect_scroll(move |_, dx, dy| {
        let (w, h) = (pic.width() as f64, pic.height() as f64);
        let (x_norm, y_norm) =
            normalize_coordinates(w, h, w / 2.0, h / 2.0).unwrap_or((32768, 32768));
        send(crate::InputGuiMessage::Scroll(ScrollEvent {
            dx: (dx * 120.0) as i16,
            dy: (dy * 120.0) as i16,
            x_norm,
            y_norm,
            timestamp_us: now_us(),
        }));
        gtk4::glib::Propagation::Stop
    });
    picture.add_controller(scroll_controller);
}

fn wire_modifiers(state: gdk::ModifierType) -> u8 {
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
    modifiers
}

fn is_reserved_key(keyval: gdk::Key) -> bool {
    keyval == gdk::Key::F11
        || keyval == gdk::Key::Escape
        || keyval == gdk::Key::Super_L
        || keyval == gdk::Key::Super_R
}

/// Forwards the keyboard to the phone while `mirror_active` is set. F11 toggles
/// fullscreen, Esc is Back, Super is Home.
pub fn attach_key_controller(window: &adw::ApplicationWindow, mirror_active: Rc<Cell<bool>>) {
    let win = window.clone();
    let active = mirror_active.clone();
    let key_controller = EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        if !active.get() {
            return gtk4::glib::Propagation::Proceed;
        }
        if keyval == gdk::Key::F11 {
            if win.is_fullscreen() {
                win.unfullscreen();
            } else {
                win.fullscreen();
            }
            return gtk4::glib::Propagation::Stop;
        }
        if keyval == gdk::Key::Escape {
            send_nav(NavAction::Back);
            return gtk4::glib::Propagation::Stop;
        }
        if keyval == gdk::Key::Super_L || keyval == gdk::Key::Super_R {
            send_nav(NavAction::Home);
            return gtk4::glib::Propagation::Stop;
        }
        send(crate::InputGuiMessage::Key(KeyEvent {
            action: KeyAction::Down,
            keycode: keyval
                .to_unicode()
                .map(|c| c as u32)
                .unwrap_or_else(|| keyval.into_glib()),
            modifiers: wire_modifiers(state),
            timestamp_us: now_us(),
        }));
        gtk4::glib::Propagation::Proceed
    });

    let active = mirror_active;
    key_controller.connect_key_released(move |_, keyval, _keycode, state| {
        if !active.get() || is_reserved_key(keyval) {
            return;
        }
        send(crate::InputGuiMessage::Key(KeyEvent {
            action: KeyAction::Up,
            keycode: keyval
                .to_unicode()
                .map(|c| c as u32)
                .unwrap_or_else(|| keyval.into_glib()),
            modifiers: wire_modifiers(state),
            timestamp_us: now_us(),
        }));
    });
    window.add_controller(key_controller);
}

/// Shows a phone notification as an in-app toast. The button runs the
/// notification's first action on the phone, or brings this window forward.
pub fn show_notification_toast(
    window: &adw::ApplicationWindow,
    toast_overlay: &adw::ToastOverlay,
    notif: hyperlink_protocol::notification::NotificationPost,
) {
    let title = match (notif.app_name.is_empty(), notif.title.is_empty()) {
        (false, false) => format!("{} · {}", notif.app_name, notif.title),
        (true, false) => notif.title.clone(),
        (false, true) => notif.app_name.clone(),
        (true, true) => "Notification from your phone".to_string(),
    };
    let toast = adw::Toast::builder().title(title).timeout(5).build();

    let (btn_label, action_id_opt) = match notif.actions.first() {
        Some(action) => (action.title.clone(), Some(action.action_id)),
        None => ("Show".to_string(), None),
    };
    toast.set_button_label(Some(&btn_label));

    let win = window.clone();
    let id = notif.id.clone();
    toast.connect_button_clicked(move |_| {
        win.present();
        if let Some(action_id) = action_id_opt {
            send(crate::InputGuiMessage::NotificationAction(
                hyperlink_protocol::notification::NotificationActionInvoke {
                    id: id.clone(),
                    action_id,
                },
            ));
        }
    });

    // Deliberately not syncing the toast's dismissal back to the phone: a toast
    // also "dismisses" when it simply times out, and that must not clear the
    // notification from the phone's shade.

    toast_overlay.add_toast(toast);
}
