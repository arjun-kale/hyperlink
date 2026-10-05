#![cfg(feature = "video")]

//! The HyperLink main window.
//!
//! One window, one question answered at a time. The home page is a stack of
//! states the user moves through: link a phone for the first time, wait for a
//! paired phone, pair a new one (with a visible countdown), pick the matching
//! code, and linked. The phone's mirrored screen is pushed on top as its own
//! page whenever the phone starts sharing it.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::video_window::{self, MirrorView};
use hyperlink_protocol::config::DeviceConfig;

/// The HyperLink mark (linux/data/icons), recolored to fit each tile.
const BRAND_MARK: &str = "com.hyperlink.Host-symbolic";

/// How long a pairing window stays open once the user starts pairing.
const PAIRING_WINDOW: Duration = Duration::from_secs(5 * 60);
/// No frames for this long while mirroring means the phone stopped sharing.
const STREAM_STALL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HomeState {
    FirstRun,
    Waiting,
    Pairing,
    ChooseCode,
    Linked,
}

impl HomeState {
    fn page_name(self) -> &'static str {
        match self {
            HomeState::FirstRun => "first-run",
            HomeState::Waiting => "waiting",
            HomeState::Pairing => "pairing",
            HomeState::ChooseCode => "choose-code",
            HomeState::Linked => "linked",
        }
    }
}

pub struct AppWindow {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    nav: adw::NavigationView,
    home_stack: gtk4::Stack,
    config_path: PathBuf,
    pc_name: String,

    countdown_label: gtk4::Label,
    pairing_generation: Cell<u64>,
    code_buttons: Vec<gtk4::Button>,
    shown_codes: RefCell<Vec<u32>>,
    pairing_reply: RefCell<Option<tokio::sync::oneshot::Sender<bool>>>,
    correct_code: Cell<u32>,

    linked_title: gtk4::Label,
    phone_connected: Cell<bool>,
    phone_name: RefCell<String>,

    mirror_page: adw::NavigationPage,
    mirror_slot: adw::Bin,
    mirror: RefCell<Option<MirrorView>>,
    mirror_active: Rc<Cell<bool>>,
    show_stats: Cell<bool>,
    last_frame: Cell<Option<std::time::Instant>>,
    told_about_background: Cell<bool>,
}

fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// Groups a 6-digit code as "123 456" — easier to compare at a glance.
fn format_code(code: u32) -> String {
    let s = format!("{code:06}");
    format!("{} {}", &s[..3], &s[3..])
}

fn label(text: &str, classes: &[&str]) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .css_classes(classes.to_vec())
        .build()
}

fn hero_icon(icon: &str, variant: Option<&str>) -> gtk4::Image {
    let image = gtk4::Image::builder()
        .icon_name(icon)
        .halign(gtk4::Align::Center)
        .css_classes(["hl-hero-icon"])
        .build();
    if let Some(v) = variant {
        image.add_css_class(v);
    }
    image
}

fn pill_button(text: &str, suggested: bool) -> gtk4::Button {
    let button = gtk4::Button::builder()
        .label(text)
        .halign(gtk4::Align::Center)
        .css_classes(["pill"])
        .build();
    if suggested {
        button.add_css_class("suggested-action");
    }
    button
}

/// A centered, width-capped column on the home backdrop.
fn home_column(children: &[&gtk4::Widget]) -> gtk4::Widget {
    let column = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .margin_top(32)
        .margin_bottom(48)
        .margin_start(24)
        .margin_end(24)
        .build();
    for child in children {
        column.append(*child);
    }
    let clamp = adw::Clamp::builder()
        .maximum_size(520)
        .child(&column)
        .build();
    gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&clamp)
        .build()
        .upcast()
}

fn spacer(height: i32) -> gtk4::Widget {
    gtk4::Box::builder().height_request(height).build().upcast()
}

/// A numbered step: "1  Install HyperLink on your phone".
fn step_row(number: u32, title: &str, detail: &str) -> gtk4::Widget {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(14)
        .build();
    row.append(
        &gtk4::Label::builder()
            .label(number.to_string())
            .valign(gtk4::Align::Start)
            .css_classes(["hl-step-number"])
            .build(),
    );
    let text = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .build();
    text.append(
        &gtk4::Label::builder()
            .label(title)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["heading"])
            .build(),
    );
    text.append(
        &gtk4::Label::builder()
            .label(detail)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label"])
            .build(),
    );
    row.append(&text);
    row.upcast()
}

/// A status chip: colored dot + short text.
fn chip(text: &str, dot_class: &str) -> gtk4::Widget {
    let chip = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk4::Align::Center)
        .css_classes(["hl-chip", "caption"])
        .build();
    let dot = gtk4::Box::builder()
        .valign(gtk4::Align::Center)
        .css_classes(["hl-dot", dot_class])
        .build();
    chip.append(&dot);
    chip.append(&gtk4::Label::new(Some(text)));
    chip.upcast()
}

fn default_mount_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("hyperlink"))
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join("HyperLink")
        })
}

fn feature_row(icon: &str, title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.add_prefix(&gtk4::Image::from_icon_name(icon));
    row
}

impl AppWindow {
    pub fn new(app: &adw::Application, config_path: PathBuf, pc_name: String) -> Rc<Self> {
        if let Some(display) = gdk::Display::default() {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(include_str!("style.css"));
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            // libadwaita 1.6+ takes the accent from CSS variables (and the
            // system accent setting) rather than @define-color. Only load these
            // there: older GTK doesn't parse custom properties.
            if adw::minor_version() >= 6 || adw::major_version() > 1 {
                let accent_vars = gtk4::CssProvider::new();
                accent_vars.load_from_string(
                    ":root { --accent-bg-color: #2366ff; --accent-fg-color: #ffffff; --accent-color: #4d86ff; }",
                );
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &accent_vars,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            // Development runs use the icons from the source tree; installed
            // builds find them in the normal icon theme paths.
            let dev_icons = concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons");
            if std::path::Path::new(dev_icons).is_dir() {
                gtk4::IconTheme::for_display(&display).add_search_path(dev_icons);
            }
        }
        gtk4::Window::set_default_icon_name("com.hyperlink.Host");

        let toasts = adw::ToastOverlay::new();
        let nav = adw::NavigationView::new();
        toasts.set_child(Some(&nav));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("HyperLink")
            .default_width(980)
            .default_height(760)
            .width_request(360)
            .height_request(480)
            .content(&toasts)
            .hide_on_close(true)
            .build();

        // Light/dark: glass needs different values per scheme (see style.css).
        let style_manager = adw::StyleManager::default();
        let sync_scheme = {
            let window = window.clone();
            move |sm: &adw::StyleManager| {
                if sm.is_dark() {
                    window.remove_css_class("hl-light");
                } else {
                    window.add_css_class("hl-light");
                }
            }
        };
        sync_scheme(&style_manager);
        style_manager.connect_dark_notify(sync_scheme);

        let home_stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(220)
            .vexpand(true)
            .build();

        let countdown_label = gtk4::Label::builder()
            .css_classes(["caption", "dim-label", "hl-countdown"])
            .build();
        let linked_title = label("Linked", &["title-1"]);
        let code_buttons: Vec<gtk4::Button> = (0..3)
            .map(|_| {
                gtk4::Button::builder()
                    .halign(gtk4::Align::Center)
                    .css_classes(["hl-code-button"])
                    .build()
            })
            .collect();

        let mirror_slot = adw::Bin::new();
        let mirror_page = adw::NavigationPage::builder()
            .title("Phone screen")
            .tag("mirror")
            .build();

        let this = Rc::new(Self {
            window,
            toasts,
            nav,
            home_stack,
            config_path,
            pc_name,
            countdown_label,
            pairing_generation: Cell::new(0),
            code_buttons,
            shown_codes: RefCell::new(Vec::new()),
            pairing_reply: RefCell::new(None),
            correct_code: Cell::new(0),
            linked_title,
            phone_connected: Cell::new(false),
            phone_name: RefCell::new(String::new()),
            mirror_page,
            mirror_slot,
            mirror: RefCell::new(None),
            mirror_active: Rc::new(Cell::new(false)),
            show_stats: Cell::new(false),
            last_frame: Cell::new(None),
            told_about_background: Cell::new(false),
        });

        this.install_actions(app);
        this.build_home_page();
        this.build_mirror_page();
        video_window::attach_key_controller(&this.window, this.mirror_active.clone());

        // Closing the window keeps HyperLink running so the phone stays linked;
        // say so once, the first time, instead of silently vanishing.
        let weak = Rc::downgrade(&this);
        this.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.on_window_hidden();
            }
            glib::Propagation::Proceed
        });

        // A stalled stream means the phone stopped sharing its screen.
        let weak = Rc::downgrade(&this);
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let (Some(mirror), Some(last)) =
                (this.mirror.borrow().as_ref(), this.last_frame.get())
            {
                mirror.set_paused(last.elapsed() > STREAM_STALL);
            }
            // The server stops waiting for an answer after a while (or the phone
            // gave up); don't leave the code picker up for a request that's gone.
            let expired = this
                .pairing_reply
                .borrow()
                .as_ref()
                .is_some_and(|reply| reply.is_closed());
            if expired {
                this.pairing_reply.take();
                this.refresh_home_after_pairing();
                this.toast("The pairing request expired. Start pairing again when you're ready.");
            }
            glib::ControlFlow::Continue
        });

        this.refresh_home();
        this
    }

    // ── Actions & menu ───────────────────────────────────────────────────

    fn install_actions(self: &Rc<Self>, app: &adw::Application) {
        let pair = gio::SimpleAction::new("pair", None);
        let weak = Rc::downgrade(self);
        pair.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.start_pairing();
            }
        });
        app.add_action(&pair);

        let prefs = gio::SimpleAction::new("preferences", None);
        let weak = Rc::downgrade(self);
        prefs.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                let weak = Rc::downgrade(&this);
                crate::preferences_window::show_preferences_window(
                    &this.window,
                    this.config_path.clone(),
                    move || {
                        if let Some(this) = weak.upgrade() {
                            this.refresh_home();
                        }
                    },
                );
            }
        });
        app.add_action(&prefs);
        app.set_accels_for_action("app.preferences", &["<Control>comma"]);

        let about = gio::SimpleAction::new("about", None);
        let weak = Rc::downgrade(self);
        about.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                let dialog = adw::AboutDialog::builder()
                    .application_name("HyperLink")
                    .application_icon("com.hyperlink.Host")
                    .developer_name("HyperLink contributors")
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments("Your Android phone and this computer, working as one — screen, notifications, clipboard and files over your own Wi-Fi. No account, no cloud.")
                    .website("https://github.com/arjun-kale/hyperlink")
                    .issue_url("https://github.com/arjun-kale/hyperlink/issues")
                    .license_type(gtk4::License::Apache20)
                    .build();
                dialog.present(Some(&this.window));
            }
        });
        app.add_action(&about);

        let stats = gio::SimpleAction::new_stateful("stats", None, &false.to_variant());
        let weak = Rc::downgrade(self);
        stats.connect_activate(move |action, _| {
            let Some(this) = weak.upgrade() else { return };
            let on = !action
                .state()
                .and_then(|s| s.get::<bool>())
                .unwrap_or(false);
            action.set_state(&on.to_variant());
            this.show_stats.set(on);
            if let Some(mirror) = this.mirror.borrow().as_ref() {
                mirror.stats_panel.set_visible(on);
            };
        });
        app.add_action(&stats);

        let fullscreen = gio::SimpleAction::new("fullscreen", None);
        let weak = Rc::downgrade(self);
        fullscreen.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                if this.window.is_fullscreen() {
                    this.window.unfullscreen();
                } else {
                    this.window.fullscreen();
                }
            }
        });
        app.add_action(&fullscreen);
    }

    fn main_menu() -> gtk4::MenuButton {
        let menu = gio::Menu::new();
        let section = gio::Menu::new();
        section.append(Some("Pair a New Phone"), Some("app.pair"));
        menu.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some("Preferences"), Some("app.preferences"));
        section.append(Some("About HyperLink"), Some("app.about"));
        menu.append_section(None, &section);
        gtk4::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main menu")
            .build()
    }

    // ── Home page ────────────────────────────────────────────────────────

    fn build_home_page(self: &Rc<Self>) {
        let header = adw::HeaderBar::new();
        header.pack_end(&Self::main_menu());

        // First run: explain the outcome, then the three steps to get there.
        let start = pill_button("Start Pairing", true);
        let weak = Rc::downgrade(self);
        start.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.start_pairing();
            }
        });
        let steps = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(18)
            .css_classes(["hl-glass"])
            .build();
        steps.append(&step_row(
            1,
            "Install HyperLink on your Android phone",
            "It's the companion app for this computer.",
        ));
        steps.append(&step_row(
            2,
            "Join the same Wi-Fi",
            "Your phone and this computer talk directly — nothing goes through the internet.",
        ));
        steps.append(&step_row(
            3,
            "Start pairing, then tap this computer on your phone",
            &format!("It appears as “{}”.", self.pc_name),
        ));
        self.home_stack.add_named(
            &home_column(&[
                hero_icon(BRAND_MARK, None).upcast_ref(),
                spacer(8).as_ref(),
                label("Link your phone", &["title-1"]).upcast_ref(),
                label(
                    "See your phone's screen, notifications, clipboard and files right here on your computer.",
                    &["body", "dim-label"],
                )
                .upcast_ref(),
                spacer(12).as_ref(),
                steps.upcast_ref(),
                spacer(12).as_ref(),
                start.upcast_ref(),
            ]),
            Some(HomeState::FirstRun.page_name()),
        );

        // Waiting for an already-paired phone.
        let pair_another = gtk4::Button::builder()
            .label("Pair a Different Phone")
            .halign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        let weak = Rc::downgrade(self);
        pair_another.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.start_pairing();
            }
        });
        self.home_stack.add_named(
            &home_column(&[
                hero_icon(BRAND_MARK, Some("idle")).upcast_ref(),
                spacer(8).as_ref(),
                label("Waiting for your phone", &["title-1"]).upcast_ref(),
                label(
                    "Open HyperLink on your phone. It connects on its own when you're both on the same Wi-Fi.",
                    &["body", "dim-label"],
                )
                .upcast_ref(),
                spacer(4).as_ref(),
                chip(&format!("This computer appears as “{}”", self.pc_name), "accent").as_ref(),
                spacer(20).as_ref(),
                pair_another.upcast_ref(),
            ]),
            Some(HomeState::Waiting.page_name()),
        );

        // Pairing window open, waiting for the phone to reach out.
        let spinner = gtk4::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .halign(gtk4::Align::Center)
            .build();
        let cancel = gtk4::Button::builder()
            .label("Cancel")
            .halign(gtk4::Align::Center)
            .css_classes(["pill"])
            .build();
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.stop_pairing();
            }
        });
        self.home_stack.add_named(
            &home_column(&[
                hero_icon(BRAND_MARK, None).upcast_ref(),
                spacer(8).as_ref(),
                label("Ready to pair", &["title-1"]).upcast_ref(),
                label(
                    &format!("On your phone, open HyperLink and tap “{}”.", self.pc_name),
                    &["body"],
                )
                .upcast_ref(),
                spacer(12).as_ref(),
                spinner.upcast_ref(),
                self.countdown_label.upcast_ref(),
                spacer(16).as_ref(),
                cancel.upcast_ref(),
            ]),
            Some(HomeState::Pairing.page_name()),
        );

        // Pick the code the phone shows. Choosing among three (rather than a
        // yes/no) forces an actual comparison — see docs/SECURITY_REVIEW.md #2.
        let codes = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .halign(gtk4::Align::Center)
            .build();
        for (i, button) in self.code_buttons.iter().enumerate() {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.on_code_chosen(i);
                }
            });
            codes.append(button);
        }
        let none_match = gtk4::Button::builder()
            .label("None of These Match")
            .halign(gtk4::Align::Center)
            .css_classes(["flat", "destructive-action"])
            .build();
        let weak = Rc::downgrade(self);
        none_match.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.finish_pairing(false, "Pairing cancelled. Nothing was saved.");
            }
        });
        self.home_stack.add_named(
            &home_column(&[
                label("Which code is on your phone?", &["title-1"]).upcast_ref(),
                label(
                    "Pick the one that exactly matches. This proves you're pairing with your own phone and not a nearby device.",
                    &["body", "dim-label"],
                )
                .upcast_ref(),
                spacer(16).as_ref(),
                codes.upcast_ref(),
                spacer(8).as_ref(),
                none_match.upcast_ref(),
            ]),
            Some(HomeState::ChooseCode.page_name()),
        );

        // Linked.
        let features = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        features.append(&feature_row(
            "preferences-system-notifications-symbolic",
            "Notifications",
            "Your phone's notifications appear on this computer",
        ));
        features.append(&feature_row(
            "edit-paste-symbolic",
            "Clipboard",
            "Copy on one device, paste on the other",
        ));
        let files_row = feature_row(
            "folder-symbolic",
            "Files",
            "Your phone's storage, browsable from your file manager",
        );
        let open_files = gtk4::Button::builder()
            .label("Open")
            .valign(gtk4::Align::Center)
            .build();
        open_files.connect_clicked(|_| {
            let launcher = gtk4::FileLauncher::new(Some(&gio::File::for_path(default_mount_dir())));
            launcher.launch(None::<&gtk4::Window>, None::<&gio::Cancellable>, |_| {});
        });
        files_row.add_suffix(&open_files);
        features.append(&files_row);
        features.append(&feature_row(
            "video-display-symbolic",
            "Phone screen",
            "Tap “Show screen on PC” on your phone to see and control it here",
        ));

        self.home_stack.add_named(
            &home_column(&[
                hero_icon(BRAND_MARK, Some("success")).upcast_ref(),
                spacer(8).as_ref(),
                self.linked_title.upcast_ref(),
                chip("Connected", "success").as_ref(),
                spacer(16).as_ref(),
                features.upcast_ref(),
            ]),
            Some(HomeState::Linked.page_name()),
        );

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&self.home_stack));
        toolbar.add_css_class("hl-backdrop");

        let page = adw::NavigationPage::builder()
            .title("HyperLink")
            .tag("home")
            .child(&toolbar)
            .build();
        self.nav.add(&page);
    }

    fn build_mirror_page(self: &Rc<Self>) {
        let header = adw::HeaderBar::new();
        let menu = gio::Menu::new();
        menu.append(Some("Show Stream Stats"), Some("app.stats"));
        menu.append(Some("Full Screen"), Some("app.fullscreen"));
        header.pack_end(
            &gtk4::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .menu_model(&menu)
                .tooltip_text("Screen options")
                .build(),
        );
        header.pack_end(
            &gtk4::Button::builder()
                .icon_name("view-fullscreen-symbolic")
                .action_name("app.fullscreen")
                .tooltip_text("Full screen (F11)")
                .build(),
        );

        let toolbar = adw::ToolbarView::builder()
            .top_bar_style(adw::ToolbarStyle::Flat)
            .build();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&self.mirror_slot));
        toolbar.add_css_class("hl-stage");

        // Full screen is for the phone, not the window chrome.
        let tb = toolbar.clone();
        self.window.connect_fullscreened_notify(move |w| {
            tb.set_reveal_top_bars(!w.is_fullscreen());
        });

        self.mirror_page.set_child(Some(&toolbar));

        let weak = Rc::downgrade(self);
        self.mirror_page.connect_shown(move |_| {
            if let Some(this) = weak.upgrade() {
                this.mirror_active.set(true);
            }
        });
        let weak = Rc::downgrade(self);
        self.mirror_page.connect_hidden(move |_| {
            if let Some(this) = weak.upgrade() {
                this.mirror_active.set(false);
                if this.window.is_fullscreen() {
                    this.window.unfullscreen();
                }
            }
        });
    }

    // ── State ────────────────────────────────────────────────────────────

    fn has_paired_devices(&self) -> bool {
        DeviceConfig::load_or_create(&self.config_path, &self.pc_name)
            .map(|c| !c.trusted_peers.is_empty())
            .unwrap_or(false)
    }

    fn show_home(&self, state: HomeState) {
        self.home_stack.set_visible_child_name(state.page_name());
    }

    /// Settles the home page on the state that matches reality, unless a
    /// pairing flow is in progress.
    fn refresh_home(&self) {
        let current = self.home_stack.visible_child_name();
        let in_pairing = matches!(current.as_deref(), Some("pairing") | Some("choose-code"));
        if in_pairing {
            return;
        }
        if self.phone_connected.get() {
            self.show_home(HomeState::Linked);
        } else if self.has_paired_devices() {
            self.show_home(HomeState::Waiting);
        } else {
            self.show_home(HomeState::FirstRun);
        }
    }

    fn start_pairing(self: &Rc<Self>) {
        self.nav.pop_to_tag("home");
        crate::connection::set_pairing_open(true);
        let generation = self.pairing_generation.get() + 1;
        self.pairing_generation.set(generation);
        self.show_home(HomeState::Pairing);

        let deadline = std::time::Instant::now() + PAIRING_WINDOW;
        let update = {
            let label = self.countdown_label.clone();
            move || {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                let secs = left.as_secs();
                label.set_label(&format!(
                    "Pairing stays open for {}:{:02}",
                    secs / 60,
                    secs % 60
                ));
                left
            }
        };
        update();
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // A newer pairing attempt (or a cancel) owns the countdown now.
            if this.pairing_generation.get() != generation
                || this.home_stack.visible_child_name().as_deref() != Some("pairing")
            {
                return glib::ControlFlow::Break;
            }
            if update().is_zero() {
                crate::connection::set_pairing_open(false);
                this.refresh_home_after_pairing();
                this.toast("Pairing timed out. Start pairing again when your phone is ready.");
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn stop_pairing(&self) {
        crate::connection::set_pairing_open(false);
        self.pairing_generation
            .set(self.pairing_generation.get() + 1);
        self.refresh_home_after_pairing();
    }

    fn refresh_home_after_pairing(&self) {
        // Leave the pairing pages explicitly; refresh_home won't while on them.
        self.show_home(HomeState::Waiting);
        self.refresh_home();
    }

    /// A phone wants to pair: show its code among two decoys.
    pub fn on_pairing_request(&self, pin: u32, reply: tokio::sync::oneshot::Sender<bool>) {
        // A previous unanswered request is superseded (and rejected).
        if let Some(old) = self.pairing_reply.replace(Some(reply)) {
            let _ = old.send(false);
        }
        self.pairing_generation
            .set(self.pairing_generation.get() + 1);
        self.correct_code.set(pin);

        let mut codes = vec![pin];
        while codes.len() < 3 {
            let decoy = (random_u64() % 1_000_000) as u32;
            if !codes.contains(&decoy) {
                codes.push(decoy);
            }
        }
        // Shuffle so the real code isn't always in the same place.
        for i in (1..codes.len()).rev() {
            let j = (random_u64() % (i as u64 + 1)) as usize;
            codes.swap(i, j);
        }
        for (button, code) in self.code_buttons.iter().zip(&codes) {
            button.set_label(&format_code(*code));
            button.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Code {}",
                format_code(*code)
            ))]);
        }
        self.shown_codes.replace(codes);

        self.nav.pop_to_tag("home");
        self.show_home(HomeState::ChooseCode);
        self.window.present();
    }

    fn on_code_chosen(&self, index: usize) {
        let chosen = self.shown_codes.borrow().get(index).copied();
        if chosen == Some(self.correct_code.get()) {
            self.finish_pairing(
                true,
                "Phone paired. It will connect automatically from now on.",
            );
        } else {
            self.finish_pairing(
                false,
                "That code didn't match, so nothing was paired. Start pairing again to retry.",
            );
        }
    }

    fn finish_pairing(&self, accept: bool, message: &str) {
        if let Some(reply) = self.pairing_reply.take() {
            let _ = reply.send(accept);
        }
        self.refresh_home_after_pairing();
        self.toast(message);
    }

    pub fn on_phone_connected(&self, name: &str) {
        self.phone_connected.set(true);
        *self.phone_name.borrow_mut() = name.to_string();
        let display = if name.starts_with("Android-Companion") {
            "your phone".to_string()
        } else {
            name.to_string()
        };
        self.linked_title
            .set_label(&format!("Linked with {display}"));
        self.mirror_page
            .set_title(&format!("Phone screen — {display}"));
        self.refresh_home();
    }

    pub fn on_phone_disconnected(&self) {
        let was_connected = self.phone_connected.replace(false);
        if self.nav.visible_page().and_then(|p| p.tag()).as_deref() == Some("mirror") {
            self.nav.pop_to_tag("home");
        }
        self.last_frame.set(None);
        self.refresh_home();
        if was_connected {
            self.toast(
                "Your phone disconnected. It reconnects automatically when it's back on Wi-Fi.",
            );
        }
    }

    // ── Mirroring ────────────────────────────────────────────────────────

    /// The phone started sharing its screen: build the view on first use and
    /// bring it to the front.
    pub fn on_stream_started(&self, paintable: &gdk::Paintable) {
        if self.mirror.borrow().is_none() {
            let view = video_window::build_mirror_view(paintable);
            view.stats_panel.set_visible(self.show_stats.get());
            self.mirror_slot.set_child(Some(&view.root));
            self.mirror.replace(Some(view));
        }
        self.last_frame.set(Some(std::time::Instant::now()));
        if let Some(mirror) = self.mirror.borrow().as_ref() {
            mirror.set_paused(false);
        }
        if self.nav.visible_page().and_then(|p| p.tag()).as_deref() != Some("mirror") {
            self.nav.push(&self.mirror_page);
        }
        self.window.present();
    }

    pub fn on_frame(&self, width: u16, height: u16, fps: f64, bitrate_kbps: u32) {
        self.last_frame.set(Some(std::time::Instant::now()));
        if let Some(mirror) = self.mirror.borrow().as_ref() {
            if video_window::set_video_dimensions(width as u32, height as u32) {
                mirror.set_aspect(width as u32, height as u32);
            }
            mirror.set_paused(false);
            if self.show_stats.get() {
                mirror.set_stats(fps, bitrate_kbps);
            }
        }
    }

    pub fn dnd_button(&self) -> Option<gtk4::ToggleButton> {
        self.mirror.borrow().as_ref().map(|m| m.dnd_button.clone())
    }

    // ── Feedback ─────────────────────────────────────────────────────────

    pub fn toast(&self, message: &str) {
        self.toasts
            .add_toast(adw::Toast::builder().title(message).timeout(5).build());
    }

    fn on_window_hidden(&self) {
        if self.told_about_background.replace(true) {
            return;
        }
        crate::dispatch_simple_desktop_notification(
            "HyperLink is still running",
            "Your phone stays linked in the background. Open HyperLink from your apps to bring this window back.",
        );
    }
}

// ── Debug-only UI snapshots ──────────────────────────────────────────────
//
// `HYPERLINK_UI_SNAPSHOTS=<dir>` (debug builds only) walks the window through
// each state in dark and light mode, saves a PNG of each, and quits. Optional
// `HYPERLINK_UI_SAMPLE=<image>` stands in for the phone's screen. Used to
// review the UI without a phone or a live session.

#[cfg(debug_assertions)]
impl AppWindow {
    pub fn capture_ui_snapshots(self: &Rc<Self>, dir: PathBuf, sample: Option<PathBuf>) {
        let _ = std::fs::create_dir_all(&dir);
        // Capture settled states, not transitions.
        if let Some(settings) = gtk4::Settings::default() {
            settings.set_gtk_enable_animations(false);
        }
        self.nav.set_animate_transitions(false);
        type Step = Box<dyn Fn(&Rc<AppWindow>)>;
        let mut steps: Vec<(String, Step)> = Vec::new();
        for scheme in ["dark", "light"] {
            let set_scheme = move || {
                adw::StyleManager::default().set_color_scheme(if scheme == "dark" {
                    adw::ColorScheme::ForceDark
                } else {
                    adw::ColorScheme::ForceLight
                });
            };
            let s = set_scheme;
            steps.push((
                format!("{scheme}-1-first-run"),
                Box::new(move |w| {
                    s();
                    w.nav.pop_to_tag("home");
                    w.show_home(HomeState::FirstRun);
                }),
            ));
            steps.push((
                format!("{scheme}-2-waiting"),
                Box::new(|w| w.show_home(HomeState::Waiting)),
            ));
            steps.push((
                format!("{scheme}-3-pairing"),
                Box::new(|w| {
                    w.start_pairing();
                }),
            ));
            steps.push((
                format!("{scheme}-4-choose-code"),
                Box::new(|w| {
                    let (tx, _rx) = tokio::sync::oneshot::channel();
                    w.on_pairing_request(482_913, tx);
                }),
            ));
            steps.push((
                format!("{scheme}-5-linked"),
                Box::new(|w| {
                    w.pairing_reply.take();
                    crate::connection::set_pairing_open(false);
                    w.phone_connected.set(true);
                    w.on_phone_connected("Galaxy S24 Ultra");
                    w.show_home(HomeState::Linked);
                }),
            ));
            let sample = sample.clone();
            steps.push((
                format!("{scheme}-6-mirror"),
                Box::new(move |w| {
                    let texture = sample
                        .as_ref()
                        .and_then(|p| gdk::Texture::from_filename(p).ok());
                    if let Some(texture) = texture {
                        video_window::set_video_dimensions(
                            texture.width() as u32,
                            texture.height() as u32,
                        );
                        let paintable: gdk::Paintable = texture.clone().upcast();
                        if w.mirror.borrow().is_none() {
                            w.on_stream_started(&paintable);
                        } else {
                            w.nav.push(&w.mirror_page);
                        }
                        w.show_stats.set(true);
                        if let Some(m) = w.mirror.borrow().as_ref() {
                            m.set_aspect(texture.width() as u32, texture.height() as u32);
                            m.stats_panel.set_visible(true);
                            m.set_stats(59.0, 6200);
                            m.set_paused(false);
                        }
                        w.last_frame
                            .set(Some(std::time::Instant::now() + Duration::from_secs(3600)));
                    }
                }),
            ));
            steps.push((
                format!("{scheme}-7-mirror-paused"),
                Box::new(|w| {
                    if let Some(m) = w.mirror.borrow().as_ref() {
                        m.stats_panel.set_visible(false);
                        m.set_paused(true);
                    }
                    w.last_frame.set(None);
                }),
            ));
        }

        let this = self.clone();
        let index = Rc::new(Cell::new(0usize));
        let steps = Rc::new(steps);
        // Apply a step, give layout and transitions time to settle, then capture.
        glib::timeout_add_local(Duration::from_millis(700), move || {
            let i = index.get();
            if i > 0 {
                let (name, _) = &steps[i - 1];
                save_widget_png(&this.window, &dir.join(format!("{name}.png")));
            }
            if i == steps.len() {
                if let Some(app) = this.window.application() {
                    app.quit();
                }
                return glib::ControlFlow::Break;
            }
            (steps[i].1)(&this);
            index.set(i + 1);
            glib::ControlFlow::Continue
        });
    }
}

#[cfg(debug_assertions)]
fn save_widget_png(widget: &impl IsA<gtk4::Widget>, path: &std::path::Path) {
    let widget = widget.as_ref();
    let paintable = gtk4::WidgetPaintable::new(Some(widget));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
    let Some(node) = snapshot.to_node() else {
        return;
    };
    let Some(renderer) = widget.native().and_then(|n| n.renderer()) else {
        return;
    };
    let texture = renderer.render_texture(&node, None);
    if let Err(e) = texture.save_to_png(path) {
        tracing::warn!(error = %e, path = %path.display(), "failed to save UI snapshot");
    }
}
