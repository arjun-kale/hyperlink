#![cfg(feature = "video")]

//! Glass surfaces for the mirror view.
//!
//! GTK's CSS has no `backdrop-filter`, so the frosted look is drawn by hand —
//! cheaply. A few times a second, [`AmbientStage`] renders the latest video
//! frame into a tiny, pre-blurred texture (~48 px wide). Drawing that texture
//! scaled up with linear filtering gives a soft, out-of-focus image for almost
//! no cost: nothing is blurred per frame, and nothing at window size.
//!
//! - [`AmbientStage`] fills the area around the phone with that image, dimmed
//!   (like a TV's bias lighting), so the phone reads as a light source instead
//!   of a hole in a dark window.
//! - [`GlassPanel`] is a container for controls floating over the stage. Behind
//!   its children it draws the same soft image (the ambient glow, and the video
//!   where it overlaps), then a tint and a light rim.

use std::time::Duration;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib, graphene, gsk};

/// Width of the soft still, in pixels. Small on purpose: see the module docs.
const STILL_WIDTH: f32 = 48.0;
/// Blur applied once, at `STILL_WIDTH` resolution.
const STILL_BLUR: f64 = 2.5;
/// How often the soft still follows the video.
const AMBIENT_REFRESH: Duration = Duration::from_millis(125);

fn is_dark() -> bool {
    libadwaita::StyleManager::default().is_dark()
}

fn stage_base() -> gdk::RGBA {
    if is_dark() {
        gdk::RGBA::new(0.043, 0.051, 0.071, 1.0)
    } else {
        gdk::RGBA::new(0.925, 0.933, 0.953, 1.0)
    }
}

/// Where `picture`'s visible video sits (it letterboxes with
/// `ContentFit::Contain`), in `target`'s coordinates.
fn video_rect_in(
    picture: &gtk4::Picture,
    target: &impl IsA<gtk4::Widget>,
) -> Option<graphene::Rect> {
    let paintable = picture.paintable()?;
    let (pw, ph) = (picture.width() as f32, picture.height() as f32);
    let (iw, ih) = (
        paintable.intrinsic_width() as f32,
        paintable.intrinsic_height() as f32,
    );
    if pw <= 0.0 || ph <= 0.0 || iw <= 0.0 || ih <= 0.0 {
        return None;
    }
    let scale = (pw / iw).min(ph / ih);
    let (rw, rh) = (iw * scale, ih * scale);
    let origin = picture.compute_point(
        target,
        &graphene::Point::new((pw - rw) / 2.0, (ph - rh) / 2.0),
    )?;
    Some(graphene::Rect::new(origin.x(), origin.y(), rw, rh))
}

fn draw_soft(snapshot: &gtk4::Snapshot, texture: &gdk::Texture, rect: &graphene::Rect) {
    snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Linear, rect);
}

// ── AmbientStage ─────────────────────────────────────────────────────────

mod stage_imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct AmbientStage {
        pub source: RefCell<Option<gtk4::Picture>>,
        /// Tiny pre-blurred copy of the latest frame (see module docs).
        pub still: RefCell<Option<gdk::Texture>>,
        pub dirty: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AmbientStage {
        const NAME: &'static str = "HlAmbientStage";
        type Type = super::AmbientStage;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
            klass.set_css_name("ambientstage");
        }
    }

    impl ObjectImpl for AmbientStage {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for AmbientStage {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let bounds =
                graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
            widget.draw_ambient(snapshot, &bounds, &*widget);
            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct AmbientStage(ObjectSubclass<stage_imp::AmbientStage>)
        @extends gtk4::Widget;
}

impl AmbientStage {
    pub fn new(source: &gtk4::Picture, child: &impl IsA<gtk4::Widget>) -> Self {
        let stage: Self = glib::Object::new();
        child.set_parent(&stage);
        stage.imp().source.replace(Some(source.clone()));

        // New frames only mark the still as stale; it's re-rendered at a fixed,
        // low rate below.
        let weak = stage.downgrade();
        let hook = move |picture: &gtk4::Picture| {
            if let Some(paintable) = picture.paintable() {
                let weak = weak.clone();
                paintable.connect_invalidate_contents(move |_| {
                    if let Some(stage) = weak.upgrade() {
                        stage.imp().dirty.set(true);
                    }
                });
            }
            if let Some(stage) = weak.upgrade() {
                stage.imp().dirty.set(true);
            }
        };
        hook(source);
        source.connect_paintable_notify(hook);

        let weak = stage.downgrade();
        glib::timeout_add_local(AMBIENT_REFRESH, move || {
            let Some(stage) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // Nothing to do while the mirror isn't on screen.
            if stage.is_mapped() && stage.imp().dirty.replace(false) {
                stage.resample();
            }
            glib::ControlFlow::Continue
        });
        stage.connect_map(|stage| stage.resample());
        stage
    }

    /// Renders the current frame into the tiny soft still.
    fn resample(&self) {
        let Some(paintable) = self
            .imp()
            .source
            .borrow()
            .as_ref()
            .and_then(|p| p.paintable())
        else {
            return;
        };
        let (iw, ih) = (
            paintable.intrinsic_width() as f32,
            paintable.intrinsic_height() as f32,
        );
        let Some(renderer) = self.native().and_then(|n| n.renderer()) else {
            return;
        };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let (w, h) = (STILL_WIDTH, (STILL_WIDTH * ih / iw).max(1.0));
        let still = paintable.current_image();
        let snapshot = gtk4::Snapshot::new();
        snapshot.push_blur(STILL_BLUR);
        still.snapshot(&snapshot, w as f64, h as f64);
        snapshot.pop();
        let Some(node) = snapshot.to_node() else {
            return;
        };
        let texture = renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, w, h)));
        self.imp().still.replace(Some(texture));
        self.queue_draw();
        // Panels frost the same image, so they follow the same refresh.
        let mut child = self.first_child();
        while let Some(widget) = child {
            queue_draw_panels(&widget);
            child = widget.next_sibling();
        }
    }

    /// The soft still, if one has been rendered yet.
    pub fn still(&self) -> Option<gdk::Texture> {
        self.imp().still.borrow().clone()
    }

    /// Draws the stage (base color + ambient glow) covering `area`, given in
    /// `target`'s coordinates. Used by the stage itself and by glass panels.
    pub fn draw_ambient(
        &self,
        snapshot: &gtk4::Snapshot,
        area: &graphene::Rect,
        target: &impl IsA<gtk4::Widget>,
    ) {
        snapshot.append_color(&stage_base(), area);
        let Some(still) = self.still() else {
            return;
        };
        let (iw, ih) = (still.width() as f32, still.height() as f32);
        let (sw, sh) = (self.width() as f32, self.height() as f32);
        if iw <= 0.0 || ih <= 0.0 || sw <= 0.0 || sh <= 0.0 {
            return;
        }
        // Cover-fit (with a little overscan, hiding the blur's soft edge) to the
        // whole stage, then map into target coords.
        let scale = (sw / iw).max(sh / ih) * 1.15;
        let (rw, rh) = (iw * scale, ih * scale);
        let Some(origin) = self.compute_point(
            target,
            &graphene::Point::new((sw - rw) / 2.0, (sh - rh) / 2.0),
        ) else {
            return;
        };
        snapshot.push_clip(area);
        snapshot.push_opacity(if is_dark() { 0.42 } else { 0.38 });
        draw_soft(
            snapshot,
            &still,
            &graphene::Rect::new(origin.x(), origin.y(), rw, rh),
        );
        snapshot.pop();
        snapshot.pop();
    }
}

fn queue_draw_panels(widget: &gtk4::Widget) {
    if widget.is::<GlassPanel>() {
        widget.queue_draw();
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        queue_draw_panels(&c);
        child = c.next_sibling();
    }
}

// ── GlassPanel ───────────────────────────────────────────────────────────

mod panel_imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct GlassPanel {
        pub source: RefCell<Option<gtk4::Picture>>,
        pub stage: RefCell<Option<glib::WeakRef<AmbientStage>>>,
        pub radius: Cell<f32>,
    }

    impl Default for GlassPanel {
        fn default() -> Self {
            Self {
                source: RefCell::new(None),
                stage: RefCell::new(None),
                radius: Cell::new(22.0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlassPanel {
        const NAME: &'static str = "HlGlassPanel";
        type Type = super::GlassPanel;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
            klass.set_css_name("glasspanel");
        }
    }

    impl ObjectImpl for GlassPanel {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for GlassPanel {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let bounds = graphene::Rect::new(0.0, 0.0, w, h);
            let radius = self.radius.get().min(w / 2.0).min(h / 2.0);
            let corner = graphene::Size::new(radius, radius);
            let rounded = gsk::RoundedRect::new(bounds, corner, corner, corner, corner);
            let dark = is_dark();

            snapshot.push_rounded_clip(&rounded);
            // Frost what's behind the panel: the ambient glow and, where the
            // panel overlaps it, an out-of-focus copy of the video.
            if let Some(stage) = self.stage.borrow().as_ref().and_then(|s| s.upgrade()) {
                stage.draw_ambient(snapshot, &bounds, &*widget);
                if let (Some(still), Some(picture)) = (stage.still(), self.source.borrow().clone())
                {
                    if let Some(rect) = video_rect_in(&picture, &*widget) {
                        draw_soft(snapshot, &still, &rect);
                    }
                }
            }

            // Tint keeps controls legible over both bright and dark content.
            let tint = if dark {
                gdk::RGBA::new(0.06, 0.07, 0.10, 0.50)
            } else {
                gdk::RGBA::new(0.99, 0.99, 1.0, 0.58)
            };
            snapshot.append_color(&tint, &bounds);
            snapshot.pop();

            // Rim: brighter along the top edge, like light catching the glass.
            let rim = |a: f32| {
                if dark {
                    gdk::RGBA::new(1.0, 1.0, 1.0, a)
                } else {
                    gdk::RGBA::new(1.0, 1.0, 1.0, (a * 2.2).min(1.0))
                }
            };
            snapshot.append_border(
                &rounded,
                &[1.0, 1.0, 1.0, 1.0],
                &[rim(0.30), rim(0.12), rim(0.07), rim(0.12)],
            );

            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct GlassPanel(ObjectSubclass<panel_imp::GlassPanel>)
        @extends gtk4::Widget;
}

impl GlassPanel {
    /// Creates a panel over `stage` that frosts the stage and `source`'s video
    /// behind `child`. It's redrawn when the stage refreshes its soft still, not
    /// on every video frame.
    pub fn new(
        source: &gtk4::Picture,
        stage: &AmbientStage,
        child: &impl IsA<gtk4::Widget>,
        radius: f32,
    ) -> Self {
        let panel: Self = glib::Object::new();
        panel.imp().radius.set(radius);
        panel.imp().stage.replace(Some(stage.downgrade()));
        panel.imp().source.replace(Some(source.clone()));
        child.set_parent(&panel);
        panel
    }
}
