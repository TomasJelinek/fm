//! Polish batch 3, the U-D13 leftover (docs/developer/acceptance/ui-polish.md): a job
//! window's footer keeps every action whole beside its dismiss button. Where its
//! actions cannot all show their captions there, they show their glyphs only, each with
//! a tooltip that names the action and with its accessible label unchanged, instead of
//! cutting an action after its glyph and first letter (Dark Touch at 320×200, Blue
//! Desktop from 14 px). Copy details, then Jobs, still leave first. The window has no
//! Details button. Measured with the fonts of
//! the platform the test runs on.
#[path = "support/window_polish.rs"]
mod polish;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ClippedImage, ComplexText, ImageItem, SimpleText},
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use midnight_ui::{JobProgressWindow, OperationRow};
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

/// A window rectangle: x, y, width, height.
type Rect = (f32, f32, f32, f32);

/// Every footer action, by its caption (which is also its accessible label).
const ACTIONS: [&str; 6] = [
    "Pause",
    "Resume",
    "Jobs",
    "Copy details",
    "Resolve",
    "Cancel operation",
];

/// The dismiss button's captions.
const DISMISS: [&str; 3] = ["Cancel operation", "Close", "Continue"];

/// Software windows whose Slint timers (tooltip delays) follow a virtual clock.
struct Clocked {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for Clocked {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

struct Fixture {
    window: JobProgressWindow,
    native: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    size: (u32, u32),
}

impl Fixture {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Clocked {
            windows: windows.clone(),
            clock: clock.clone(),
        }))
        .unwrap();
        let window = JobProgressWindow::new().unwrap();
        window.show().unwrap();
        let native = windows.borrow()[0].clone();
        Self {
            window,
            native,
            clock,
            size: (640, 380),
        }
    }

    /// Show `job` in an appearance (theme, density, font size) at `size`, or at the
    /// window's own minimum size.
    fn show(&mut self, job: &OperationRow, asking: bool, look: Look, size: Option<(u32, u32)>) {
        let (_, scheme, density, font) = look;
        self.window
            .invoke_apply_preferences(scheme, density, "".into(), font, font);
        self.window.set_job(job.clone());
        self.window.set_cancel_confirmation(asking);
        polish_layout(&self.native, 900, 700);
        self.size = size.unwrap_or_else(|| polish::minimum_size(self.native.window()));
        self.draw();
    }

    fn draw(&self) {
        slint::platform::update_timers_and_animations();
        let (width, height) = self.size;
        let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
        self.native
            .set_size(slint::PhysicalSize::new(width, height));
        self.native.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.native.window().request_redraw();
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
    }

    fn advance(&self, by: Duration) {
        self.clock.set(self.clock.get() + by);
        slint::platform::update_timers_and_animations();
    }
}

fn polish_layout(native: &MinimalSoftwareWindow, width: u32, height: u32) {
    native.set_size(slint::PhysicalSize::new(width, height));
    native.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    WindowInner::from_pub(native.window()).ensure_tree_instantiated();
}

/// An appearance: its name, theme, density and font size.
type Look = (&'static str, i32, i32, i32);

/// Blue Desktop and Dark Touch, at the gate's fonts and at the larger ones where the
/// captions stopped fitting.
const LOOKS: [Look; 7] = [
    ("Blue Desktop", 0, 0, 12),
    ("Blue Desktop", 0, 0, 14),
    ("Blue Desktop", 0, 0, 16),
    ("Blue Desktop", 0, 0, 20),
    ("Dark Touch", 1, 1, 15),
    ("Dark Touch", 1, 1, 18),
    ("Dark Touch", 1, 1, 20),
];

/// A running Copy job's row, as the presenter fills the parts the footer reads.
fn copy_job(edit: impl FnOnce(&mut OperationRow)) -> OperationRow {
    let mut row = OperationRow {
        identity: "copy-job:1".into(),
        title: "Copy 3 items".into(),
        state: "Copying".into(),
        summary: "1 of 3 files copied".into(),
        detail: "Source\nD:\\文档\\source\nDestination\nE:\\target".into(),
        active: true,
        cancellable: true,
        pausable: true,
        determinate: true,
        progress: 0.4,
        progress_label: "40% · 4 MB of 10 MB".into(),
        ..Default::default()
    };
    edit(&mut row);
    row
}

/// The job states whose footers differ: what the job is doing, its row, and whether the
/// window asks whether to cancel it.
fn states() -> Vec<(&'static str, OperationRow, bool)> {
    vec![
        ("running", copy_job(|_| {}), false),
        (
            "paused",
            copy_job(|row| {
                row.paused = true;
                row.pause_requested = true;
            }),
            false,
        ),
        (
            "waiting for a decision",
            copy_job(|row| row.attention = true),
            false,
        ),
        (
            "running, not pausable",
            copy_job(|row| row.pausable = false),
            false,
        ),
        (
            "finished with errors",
            copy_job(|row| {
                row.active = false;
                row.pausable = false;
                row.cancellable = false;
                row.failed = true;
                row.state = "Completed with errors".into();
            }),
            false,
        ),
        ("asking to cancel", copy_job(|_| {}), true),
    ]
}

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

/// What the user sees of `item`: its bounds cut by the window and every clipping
/// ancestor (the footer's action scroller among them).
fn shown_part(item: &ItemRc, (width, height): (u32, u32)) -> Rect {
    let mut shown = intersect(
        polish::bounds(item),
        (0.0, 0.0, width as f32, height as f32),
    );
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            shown = intersect(shown, polish::bounds(&ancestor));
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    shown
}

fn walk(item: &ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(item);
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        walk(&current, found);
    }
}

/// The caption a button draws: its text, the width it needs and the width it has.
fn caption(button: &ItemRc) -> Option<(String, f32, f32)> {
    let adapter = button.window_adapter()?;
    let mut found = None;
    walk(button, &mut |item| {
        if found.is_some() || !item.is_visible() {
            return;
        }
        let text = if let Some(text) = item.downcast::<SimpleText>() {
            text.as_pin_ref().text().to_string()
        } else if let Some(text) = item.downcast::<ComplexText>() {
            text.as_pin_ref().text().to_string()
        } else {
            return;
        };
        if text.chars().any(char::is_alphanumeric) {
            let needed = item
                .borrow()
                .as_ref()
                .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
                .preferred;
            found = Some((text, needed, item.geometry().width()));
        }
    });
    found
}

/// Whether a button draws a glyph.
fn has_glyph(button: &ItemRc) -> bool {
    let mut glyph = false;
    walk(button, &mut |item| {
        glyph |= (item.downcast::<ImageItem>().is_some()
            || item.downcast::<ClippedImage>().is_some())
            && item.is_visible()
            && item.geometry().width() > 1.0;
    });
    glyph
}

/// The shown footer buttons: actions and the dismiss button, left to right.
fn footer_buttons(native: &MinimalSoftwareWindow) -> Vec<(String, ItemRc)> {
    let root = ItemRc::new_root(WindowInner::from_pub(native.window()).component());
    let mut found = Vec::new();
    walk(&root, &mut |item| {
        if item.accessible_role() == AccessibleRole::Button && item.is_visible() {
            let name = label(item);
            if ACTIONS.contains(&name.as_str()) || DISMISS.contains(&name.as_str()) {
                found.push((name, item.clone()));
            }
        }
    });
    found.sort_by(|(_, a), (_, b)| polish::bounds(a).0.total_cmp(&polish::bounds(b).0));
    found
}

/// What is cut in the footer: a button that the scroller or the window shows only in
/// part, a caption that does not fit its button, or a button with neither caption
/// nor glyph.
fn cut_footer(fixture: &Fixture) -> Vec<String> {
    let mut findings = Vec::new();
    for (name, button) in footer_buttons(&fixture.native) {
        let whole = polish::bounds(&button);
        let shown = shown_part(&button, fixture.size);
        if shown.2 + 0.5 < whole.2 || shown.3 + 0.5 < whole.3 {
            findings.push(format!(
                "{name:?} shows {:.0}×{:.0} of {:.0}×{:.0}px",
                shown.2, shown.3, whole.2, whole.3
            ));
        }
        match caption(&button) {
            Some((text, needed, has)) if needed > has + 1.0 => findings.push(format!(
                "{name:?}: caption {text:?} needs {needed:.0}px, has {has:.0}px"
            )),
            Some(_) => {}
            None if !has_glyph(&button) => {
                findings.push(format!("{name:?} shows neither a caption nor a glyph"))
            }
            None => {}
        }
    }
    findings
}

#[test]
fn job_window_footers_keep_every_action_whole_at_their_minimum_size() {
    let mut fixture = Fixture::new();
    let mut findings = Vec::new();
    for (what, job, asking) in states() {
        for look in LOOKS {
            fixture.show(&job, asking, look, None);
            let (width, height) = fixture.size;
            findings.extend(cut_footer(&fixture).into_iter().map(|finding| {
                format!(
                    "{what}, {} {}px at {width}×{height}: {finding}",
                    look.0, look.3
                )
            }));
        }
    }
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn footer_actions_keep_their_captions_where_they_fit() {
    let mut fixture = Fixture::new();
    for (what, job, asking) in states() {
        for look in [LOOKS[0], LOOKS[4]] {
            fixture.show(&job, asking, look, Some((640, 380)));
            for (name, button) in footer_buttons(&fixture.native) {
                assert!(
                    caption(&button).is_some(),
                    "{what}, {} at 640×380: {name:?} shows its glyph only",
                    look.0
                );
            }
        }
    }
}

/// The texts of the tooltip that resting the pointer on `item` shows.
fn tooltip_of(fixture: &Fixture, item: &ItemRc) -> Vec<String> {
    let window = fixture.native.window();
    window.dispatch_event(WindowEvent::PointerExited);
    fixture.advance(Duration::from_millis(200));
    let (x, y, w, h) = polish::bounds(item);
    window.dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0),
    });
    fixture.advance(Duration::from_millis(1200));
    fixture.draw();
    let mut texts = Vec::new();
    for popup in WindowInner::from_pub(window).active_popups().iter() {
        if matches!(popup.window_kind, WindowKind::ToolTip) {
            walk(&ItemRc::new_root(popup.component.clone()), &mut |item| {
                if let Some(text) = item.downcast::<ComplexText>() {
                    texts.push(text.as_pin_ref().text().to_string());
                } else if let Some(text) = item.downcast::<SimpleText>() {
                    texts.push(text.as_pin_ref().text().to_string());
                }
            });
        }
    }
    window.dispatch_event(WindowEvent::PointerExited);
    fixture.advance(Duration::from_millis(200));
    texts.retain(|text| !text.trim().is_empty());
    texts
}

#[test]
fn glyph_only_actions_name_their_action_in_a_tooltip_and_keep_their_labels() {
    let mut fixture = Fixture::new();
    let mut glyphs = 0;
    for look in LOOKS {
        for (what, job, expected) in [
            ("running", copy_job(|_| {}), &["Pause"][..]),
            (
                "paused",
                copy_job(|row| {
                    row.paused = true;
                    row.pause_requested = true;
                }),
                &["Resume"][..],
            ),
            (
                "waiting for a decision",
                copy_job(|row| row.attention = true),
                &["Pause", "Resolve"][..],
            ),
        ] {
            fixture.show(&job, false, look, None);
            let what = format!("{what}, {} {}px at {:?}", look.0, look.3, fixture.size);
            let buttons = footer_buttons(&fixture.native);
            let names: Vec<&str> = buttons
                .iter()
                .map(|(name, _)| name.as_str())
                .filter(|name| *name != "Jobs")
                .collect();
            let mut wanted = expected.to_vec();
            wanted.push("Cancel operation");
            assert_eq!(
                names, wanted,
                "{what}: the footer, left to right, no Details"
            );
            for (name, button) in &buttons {
                if name == "Cancel operation" {
                    assert!(
                        caption(button).is_some(),
                        "{what}: the dismiss button keeps its caption"
                    );
                    continue;
                }
                if caption(button).is_some() {
                    continue;
                }
                // Where the caption does not fit, the glyph stays and the tooltip names it.
                glyphs += 1;
                assert!(has_glyph(button), "{what}: {name:?} shows a glyph");
                let tip = tooltip_of(&fixture, button);
                assert!(
                    tip.iter()
                        .any(|text| text.lines().next() == Some(name.as_str())),
                    "{what}: {name:?}'s tooltip names the action on its first line: {tip:?}"
                );
            }
        }
    }
    assert!(glyphs > 0, "some minimum size shows glyph-only actions");
    // An action still works, and Pause turning into Resume moves no other action.
    let dark_touch = LOOKS[6];
    fixture.show(&copy_job(|_| {}), false, dark_touch, None);
    let paused = Rc::new(Cell::new(None));
    let seen = paused.clone();
    fixture
        .window
        .on_set_paused(move |value| seen.set(Some(value)));
    let places = |fixture: &Fixture| -> Vec<(String, f32)> {
        footer_buttons(&fixture.native)
            .iter()
            .map(|(name, item)| (name.clone(), polish::bounds(item).0))
            .collect()
    };
    let before = places(&fixture);
    let pause = footer_buttons(&fixture.native)
        .into_iter()
        .find(|(name, _)| name == "Pause")
        .expect("Pause")
        .1;
    let (x, y, w, h) = polish::bounds(&pause);
    let position = slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0);
    let button = slint::platform::PointerEventButton::Left;
    let window = fixture.native.window();
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(paused.get(), Some(true), "Pause asks to pause");
    fixture.window.set_job(copy_job(|row| {
        row.paused = true;
        row.pause_requested = true;
    }));
    fixture.draw();
    let after = places(&fixture);
    assert_eq!(after.len(), before.len());
    for ((name, x), (was, at)) in after.iter().zip(&before) {
        if was == "Pause" {
            assert_eq!(name, "Resume");
        }
        if was == "Pause" || was == "Cancel operation" {
            assert_eq!(x, at, "{was} stays in place");
        }
    }
}
