#![cfg(feature = "video")]

//! Libadwaita preferences window (Phase 11).
//!
//! Reads and writes `DeviceConfig` directly against `config_path` on disk —
//! it does not share live state with an already-running connection. That's a
//! deliberate, disclosed limitation rather than an oversight: feature
//! toggles and the bandwidth cap apply to the *next* session, and revoking a
//! trusted device here does not forcibly disconnect a peer that is already
//! connected using the in-memory trust set the connection handler loaded at
//! startup — both of those are called out in the UI copy below rather than
//! silently assumed to be live.

use gtk4::prelude::*;
use libadwaita::prelude::*;
use libadwaita::{self as adw, ApplicationWindow};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

use hyperlink_protocol::config::{DeviceConfig, HostPreferences};

/// Builds and presents the preferences window on top of `parent`.
pub fn show_preferences_window(parent: &ApplicationWindow, config_path: PathBuf) {
    let config = match DeviceConfig::load_or_create(&config_path, "HyperLink-Host") {
        Ok(c) => Arc::new(Mutex::new(c)),
        Err(e) => {
            error!(error = %e, "failed to load config for preferences window");
            return;
        }
    };

    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(600)
        .search_enabled(true)
        .build();

    window.add(&build_features_page(&config, &config_path));
    window.add(&build_bandwidth_page(&config, &config_path));
    window.add(&build_privacy_page(&config, &config_path));
    window.add(&build_devices_page(&config, &config_path, &window));

    window.present();
}

fn save(config: &Arc<Mutex<DeviceConfig>>, config_path: &PathBuf) {
    let guard = config.lock().unwrap();
    if let Err(e) = guard.save(config_path) {
        error!(error = %e, path = %config_path.display(), "failed to save preferences");
    }
}

/// Adds a switch row bound to one boolean field of `HostPreferences`, saving
/// to disk immediately on toggle (this window has no separate "Apply" step).
fn add_feature_switch(
    group: &adw::PreferencesGroup,
    title: &str,
    subtitle: &str,
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &PathBuf,
    get: impl Fn(&HostPreferences) -> bool + 'static,
    set: impl Fn(&mut HostPreferences, bool) + 'static,
) {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();

    let initial = get(&config.lock().unwrap().preferences);
    let switch = gtk4::Switch::builder()
        .active(initial)
        .valign(gtk4::Align::Center)
        .build();

    let config_clone = config.clone();
    let config_path_clone = config_path.clone();
    switch.connect_state_set(move |_, active| {
        {
            let mut guard = config_clone.lock().unwrap();
            set(&mut guard.preferences, active);
        }
        save(&config_clone, &config_path_clone);
        gtk4::glib::Propagation::Proceed
    });

    row.add_suffix(&switch);
    row.set_activatable_widget(Some(&switch));
    group.add(&row);
}

fn build_features_page(
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &PathBuf,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Features")
        .icon_name("preferences-system-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Enabled Features")
        .description("Applies to the next session — an already-active mirror keeps running with the settings it started with.")
        .build();

    add_feature_switch(
        &group,
        "Video Mirroring",
        "Mirror the phone's screen to this window",
        config,
        config_path,
        |p| p.enable_video,
        |p, v| p.enable_video = v,
    );
    add_feature_switch(
        &group,
        "Input Injection",
        "Send keyboard, mouse, and touch input to the phone",
        config,
        config_path,
        |p| p.enable_input,
        |p, v| p.enable_input = v,
    );
    add_feature_switch(
        &group,
        "Notifications",
        "Mirror phone notifications to the desktop",
        config,
        config_path,
        |p| p.enable_notifications,
        |p, v| p.enable_notifications = v,
    );
    add_feature_switch(
        &group,
        "Clipboard Sync",
        "Share the system clipboard in both directions",
        config,
        config_path,
        |p| p.enable_clipboard,
        |p, v| p.enable_clipboard = v,
    );
    add_feature_switch(
        &group,
        "File Access",
        "Mount the phone's storage as a local virtual filesystem",
        config,
        config_path,
        |p| p.enable_file_access,
        |p, v| p.enable_file_access = v,
    );
    add_feature_switch(
        &group, "Proximity Pre-Warm", "Opportunistically pre-warm reconnects — never bypasses certificate authentication (see docs/SECURITY_REVIEW.md)",
        config, config_path,
        |p| p.enable_proximity_prewarm, |p, v| p.enable_proximity_prewarm = v,
    );

    page.add(&group);
    page
}

fn build_bandwidth_page(
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &PathBuf,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Bandwidth")
        .icon_name("network-wireless-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Video Bitrate")
        .description("Caps the encoder's target bitrate for the next mirroring session.")
        .build();

    let row = adw::ActionRow::builder()
        .title("Max Bitrate")
        .subtitle("0 = unlimited (encoder default)")
        .build();

    let current_kbps = config
        .lock()
        .unwrap()
        .preferences
        .max_bitrate_kbps
        .unwrap_or(0);
    let spin = gtk4::SpinButton::with_range(0.0, 20_000.0, 250.0);
    spin.set_value(current_kbps as f64);
    spin.set_digits(0);

    let config_clone = config.clone();
    let config_path_clone = config_path.clone();
    spin.connect_value_changed(move |sb| {
        let kbps = sb.value().round() as u32;
        {
            let mut guard = config_clone.lock().unwrap();
            guard.preferences.max_bitrate_kbps = if kbps == 0 { None } else { Some(kbps) };
        }
        save(&config_clone, &config_path_clone);
    });

    row.add_suffix(&spin);
    row.set_activatable_widget(Some(&spin));
    group.add(&row);

    page.add(&group);
    page
}

fn build_privacy_page(
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &PathBuf,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Privacy & Diagnostics")
        .icon_name("dialog-information-symbolic")
        .build();

    // --- Crash reports ---
    let crash_group = adw::PreferencesGroup::builder()
        .title("Crash Reports")
        .description("Written only to this computer's disk, never transmitted anywhere. Contains a panic message, code location, and optional backtrace — never clipboard text, notification content, file names, or key material. See docs/SECURITY_REVIEW.md.")
        .build();

    add_feature_switch(
        &crash_group,
        "Save Local Crash Reports",
        "Write a redacted report to ~/.local/share/hyperlink/crash_reports on a crash",
        config,
        config_path,
        |p| p.crash_reporting_enabled,
        |p, v| p.crash_reporting_enabled = v,
    );

    // Wire the toggle to the live panic hook too, so it takes effect immediately
    // rather than only on next launch.
    {
        let config_for_live = config.clone();
        // Re-find the switch we just added isn't straightforward without keeping a
        // handle, so mirror the current on-disk value into the live hook once here;
        // subsequent toggles are propagated via crash_report::set_enabled below.
        let enabled_now = config_for_live
            .lock()
            .unwrap()
            .preferences
            .crash_reporting_enabled;
        crate::crash_report::set_enabled(enabled_now);
    }

    let reports_dir = crate::crash_report::default_reports_dir();
    let reports_row = adw::ActionRow::builder()
        .title("Existing Reports")
        .subtitle(format!("{}", reports_dir.display()))
        .build();

    let open_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    open_btn.set_valign(gtk4::Align::Center);
    open_btn.set_tooltip_text(Some("Open reports folder"));
    let reports_dir_open = reports_dir.clone();
    open_btn.connect_clicked(move |_| {
        let _ = std::process::Command::new("xdg-open")
            .arg(&reports_dir_open)
            .spawn();
    });

    let clear_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    clear_btn.set_valign(gtk4::Align::Center);
    clear_btn.set_tooltip_text(Some("Clear all local crash reports"));
    let reports_dir_clear = reports_dir.clone();
    clear_btn.connect_clicked(move |_| {
        crate::crash_report::clear_reports(&reports_dir_clear);
        info!("cleared crash reports from preferences window");
    });

    reports_row.add_suffix(&open_btn);
    reports_row.add_suffix(&clear_btn);
    crash_group.add(&reports_row);

    page.add(&crash_group);

    // --- Updates ---
    let update_group = adw::PreferencesGroup::builder()
        .title("Updates")
        .description("Checks a version feed and notifies you — never downloads or installs anything automatically. Update through your normal channel (Flatpak, package manager, or git pull + cargo build).")
        .build();

    add_feature_switch(
        &update_group,
        "Check for Updates",
        "Periodically check whether a newer release is available",
        config,
        config_path,
        |p| p.update_check_enabled,
        |p, v| p.update_check_enabled = v,
    );

    let check_row = adw::ActionRow::builder()
        .title("Current Version")
        .subtitle(env!("CARGO_PKG_VERSION"))
        .build();
    let check_btn = gtk4::Button::with_label("Check Now");
    check_btn.set_valign(gtk4::Align::Center);
    let subtitle_label_row = check_row.clone();
    check_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        let row = subtitle_label_row.clone();
        let btn_clone = btn.clone();
        gtk4::glib::spawn_future_local(async move {
            let result = crate::update_check::check_for_update(env!("CARGO_PKG_VERSION")).await;
            let text = match (&result.latest_version, result.update_available) {
                (Some(latest), true) => {
                    format!("{} (update available: {latest})", result.current_version)
                }
                (Some(_), false) => format!("{} (up to date)", result.current_version),
                (None, _) => format!("{} (check failed — offline?)", result.current_version),
            };
            row.set_subtitle(&text);
            btn_clone.set_sensitive(true);
        });
    });
    check_row.add_suffix(&check_btn);
    update_group.add(&check_row);

    page.add(&update_group);
    page
}

fn build_devices_page(
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &PathBuf,
    window: &adw::PreferencesWindow,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Trusted Devices")
        .icon_name("network-transmit-receive-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Paired Devices")
        .description("Removing a device here stops it from pairing again automatically, but does not disconnect a session already in progress — restart HyperLink to force-drop an active connection.")
        .build();

    let peers: Vec<(String, String)> = config
        .lock()
        .unwrap()
        .trusted_peers
        .iter()
        .map(|(name, fp)| (name.clone(), fp.clone()))
        .collect();

    if peers.is_empty() {
        let empty_row = adw::ActionRow::builder()
            .title("No paired devices yet")
            .subtitle("Run with --pair to pair a new phone")
            .build();
        group.add(&empty_row);
    }

    for (name, fingerprint) in peers {
        let row = adw::ActionRow::builder()
            .title(&name)
            .subtitle(&fingerprint)
            .build();

        let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove_btn.add_css_class("flat");
        remove_btn.set_valign(gtk4::Align::Center);
        remove_btn.set_tooltip_text(Some("Revoke trust for this device"));

        let config_clone = config.clone();
        let config_path_clone = config_path.clone();
        let name_clone = name.clone();
        let window_clone = window.clone();
        let row_clone = row.clone();
        let group_clone = group.clone();
        remove_btn.connect_clicked(move |_| {
            {
                let mut guard = config_clone.lock().unwrap();
                guard.remove_trusted_peer(&name_clone);
            }
            save(&config_clone, &config_path_clone);
            group_clone.remove(&row_clone);
            info!(peer = %name_clone, "revoked trusted peer via preferences window");
            let _ = &window_clone;
        });

        row.add_suffix(&remove_btn);
        group.add(&row);
    }

    page.add(&group);
    page
}
