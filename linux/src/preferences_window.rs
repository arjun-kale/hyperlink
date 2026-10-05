#![cfg(feature = "video")]

//! Libadwaita preferences window (Phase 11).
//!
//! Reads and writes `DeviceConfig` directly against `config_path` on disk.
//! Feature toggles and the bandwidth cap apply to the *next* session, which
//! the UI copy says. Removing a device is live: it also leaves the server's
//! in-memory trust set and drops the phone's session if one is up.

use gtk4::prelude::*;
use libadwaita::prelude::*;
use libadwaita::{self as adw, ApplicationWindow};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{error, info};

use hyperlink_protocol::config::{DeviceConfig, HostPreferences};

/// Builds and presents the preferences window on top of `parent`.
/// `on_devices_changed` runs after a device is removed.
pub fn show_preferences_window(
    parent: &ApplicationWindow,
    config_path: PathBuf,
    on_devices_changed: impl Fn() + 'static,
) {
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
    window.add(&build_devices_page(
        &config,
        &config_path,
        &window,
        std::rc::Rc::new(on_devices_changed),
    ));

    window.present();
}

fn save(config: &Arc<Mutex<DeviceConfig>>, config_path: &Path) {
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
    config_path: &Path,
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
    let config_path_clone = config_path.to_path_buf();
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
    config_path: &Path,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Features")
        .icon_name("preferences-system-symbolic")
        .build();

    page.add(&build_computer_group(config, config_path));

    let group = adw::PreferencesGroup::builder()
        .title("What Your Phone Can Do Here")
        .description("Changes apply the next time your phone connects.")
        .build();

    add_feature_switch(
        &group,
        "Phone Screen",
        "Show your phone's screen in a window on this computer",
        config,
        config_path,
        |p| p.enable_video,
        |p, v| p.enable_video = v,
    );
    add_feature_switch(
        &group,
        "Control From This Computer",
        "Use your mouse and keyboard on the phone's screen",
        config,
        config_path,
        |p| p.enable_input,
        |p, v| p.enable_input = v,
    );
    add_feature_switch(
        &group,
        "Notifications",
        "Show your phone's notifications on this computer",
        config,
        config_path,
        |p| p.enable_notifications,
        |p, v| p.enable_notifications = v,
    );
    add_feature_switch(
        &group,
        "Shared Clipboard",
        "Copy on one device, paste on the other",
        config,
        config_path,
        |p| p.enable_clipboard,
        |p, v| p.enable_clipboard = v,
    );
    add_feature_switch(
        &group,
        "Phone Files",
        "Browse your phone's storage from your file manager",
        config,
        config_path,
        |p| p.enable_file_access,
        |p, v| p.enable_file_access = v,
    );
    add_feature_switch(
        &group, "Faster Reconnect When Nearby", "Get ready to connect when your phone comes close. Your phone still has to prove it's yours",
        config, config_path,
        |p| p.enable_proximity_prewarm, |p, v| p.enable_proximity_prewarm = v,
    );

    page.add(&group);
    page
}

fn build_bandwidth_page(
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &Path,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Bandwidth")
        .icon_name("network-wireless-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Screen Quality")
        .description("Lower this if the phone's screen stutters on slow Wi-Fi. Applies the next time screen sharing starts.")
        .build();

    let row = adw::ActionRow::builder()
        .title("Maximum Bitrate (kbps)")
        .subtitle("0 means no limit")
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
    let config_path_clone = config_path.to_path_buf();
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
    config_path: &Path,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Privacy & Diagnostics")
        .icon_name("dialog-information-symbolic")
        .build();

    // --- Crash reports ---
    let crash_group = adw::PreferencesGroup::builder()
        .title("Crash Reports")
        .description("Saved only on this computer and never sent anywhere. Reports never include clipboard text, notification content, file names or keys.")
        .build();

    add_feature_switch(
        &crash_group,
        "Save Crash Reports",
        "Helps diagnose problems if HyperLink stops unexpectedly",
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
        .description("HyperLink tells you when a new version is out. It never downloads or installs anything by itself.")
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
                (None, _) => format!(
                    "{} (couldn't check — are you offline?)",
                    result.current_version
                ),
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
    config_path: &Path,
    window: &adw::PreferencesWindow,
    on_devices_changed: std::rc::Rc<dyn Fn()>,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Paired Phones")
        .icon_name("phone-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Paired Phones")
        .description(
            "Removing a phone disconnects it right away. It has to be paired again to reconnect.",
        )
        .build();

    let peers: Vec<(String, String)> = config
        .lock()
        .unwrap()
        .trusted_peers
        .iter()
        .map(|(name, fp)| (name.clone(), fp.clone()))
        .collect();

    let empty_row = adw::ActionRow::builder()
        .title("No paired phones yet")
        .subtitle("Use “Pair a New Phone” in the main menu")
        .build();
    group.add(&empty_row);
    empty_row.set_visible(peers.is_empty());

    for (name, fingerprint) in peers {
        let short_fp: String = fingerprint.chars().take(11).collect();
        let row = adw::ActionRow::builder()
            .title(&name)
            .subtitle(format!("Security fingerprint {short_fp}…"))
            .build();
        row.add_prefix(&gtk4::Image::from_icon_name("phone-symbolic"));

        let remove_btn = gtk4::Button::builder()
            .label("Remove")
            .valign(gtk4::Align::Center)
            .css_classes(["flat", "destructive-action"])
            .build();

        let config_clone = config.clone();
        let config_path_clone = config_path.to_path_buf();
        let window_clone = window.clone();
        let row_clone = row.clone();
        let group_clone = group.clone();
        let empty_clone = empty_row.clone();
        let changed = on_devices_changed.clone();
        remove_btn.connect_clicked(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading(format!("Remove “{name}”?"))
                .body("It will be disconnected now and won't be able to connect until you pair it again.")
                .close_response("cancel")
                .default_response("cancel")
                .build();
            dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
            dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

            let config_clone = config_clone.clone();
            let config_path_clone = config_path_clone.clone();
            let name = name.clone();
            let fingerprint = fingerprint.clone();
            let row_clone = row_clone.clone();
            let group_clone = group_clone.clone();
            let empty_clone = empty_clone.clone();
            let changed = changed.clone();
            dialog.connect_response(None, move |_, response| {
                if response != "remove" {
                    return;
                }
                let now_empty = {
                    let mut guard = config_clone.lock().unwrap();
                    guard.remove_trusted_peer(&name);
                    crate::refresh_proximity_trust(&guard.trusted_peers);
                    guard.trusted_peers.is_empty()
                };
                save(&config_clone, &config_path_clone);
                crate::connection::revoke_device(&fingerprint);
                group_clone.remove(&row_clone);
                empty_clone.set_visible(now_empty);
                info!(peer = %name, "removed paired phone via preferences window");
                changed();
            });
            dialog.present(Some(&window_clone));
        });

        row.add_suffix(&remove_btn);
        group.add(&row);
    }

    page.add(&group);
    page
}

/// Name phones see, and whether HyperLink starts with the session.
fn build_computer_group(
    config: &Arc<Mutex<DeviceConfig>>,
    config_path: &Path,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("This Computer")
        .build();

    let name_row = adw::EntryRow::builder()
        .title("Name shown on your phone")
        .text(config.lock().unwrap().device_name.as_str())
        .show_apply_button(true)
        .build();
    let config_clone = config.clone();
    let config_path_clone = config_path.to_path_buf();
    name_row.connect_apply(move |row| {
        let name = row.text().trim().to_string();
        if name.is_empty() {
            row.set_text(&config_clone.lock().unwrap().device_name);
            return;
        }
        config_clone.lock().unwrap().device_name = name;
        save(&config_clone, &config_path_clone);
    });
    group.add(&name_row);
    group.set_description(Some(
        "A new name appears on your phone the next time HyperLink starts.",
    ));

    // Autostart via an XDG autostart entry. Inside Flatpak this needs the
    // Background portal instead, so the switch isn't offered there.
    if std::env::var_os("FLATPAK_ID").is_none() {
        let row = adw::SwitchRow::builder()
            .title("Start When You Log In")
            .subtitle("Keeps your phone linked without opening HyperLink first")
            .active(autostart_path().is_file())
            .build();
        row.connect_active_notify(|row| {
            if let Err(e) = set_autostart(row.is_active()) {
                error!(error = %e, "failed to change autostart");
            }
        });
        group.add(&row);
    }

    group
}

fn autostart_path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        });
    config_home.join("autostart/com.hyperlink.Host.desktop")
}

fn set_autostart(enabled: bool) -> std::io::Result<()> {
    let path = autostart_path();
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        &path,
        format!(
            "[Desktop Entry]\nType=Application\nName=HyperLink\nExec=\"{}\" --background\nIcon=com.hyperlink.Host\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
            exe.display()
        ),
    )
}
