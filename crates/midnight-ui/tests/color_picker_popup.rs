//! A color chooser is a separate, scrollable popup within the Options session.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::{
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::{Command, Event};
use midnight_ui::{ColorOptionsView, Presenter, install, refresh};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

struct Harness {
    ui: ui::TestUi,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let state = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let observed = sent.clone();
        install(
            &ui.window,
            state.clone(),
            Rc::new(move |command| {
                observed.borrow_mut().push(command);
                true
            }),
        );
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        refresh(&ui.window, &state.borrow());
        let mut h = Self { ui, sent };
        h.ui.window.invoke_show_options();
        h.ui.window.set_options_topic_index(8);
        h.ui.draw();
        h
    }

    fn open(&mut self, field: &str) {
        ui::click_popup(
            &self.ui.window,
            &format!("Choose {field}"),
            AccessibleRole::Button,
        );
        self.ui.draw();
        assert_eq!(
            self.popups(),
            2,
            "the chooser must not expand the Options form"
        );
    }

    fn popups(&self) -> usize {
        WindowInner::from_pub(self.ui.window.window())
            .active_popups()
            .iter()
            .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .count()
    }

    /// Centre of a popup button that is inside the window and every scrolling
    /// ancestor's clip (a control under the ScrollView clip is not "in view").
    fn inside(&self, label: &str) -> LogicalPosition {
        use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable};
        let item = ui::popup_item(&self.ui.window, label, AccessibleRole::Button);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        assert!(
            origin.x >= 0.
                && origin.y >= 0.
                && origin.x + rect.width() <= self.ui.width as f32 + 0.5
                && origin.y + rect.height() <= self.ui.height as f32 + 0.5,
            "{label} is clipped: {origin:?}, {rect:?}"
        );
        let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        while let Some(ancestor) = parent {
            if ancestor.downcast::<Flickable>().is_some() {
                let clip = ancestor.geometry();
                let at = ancestor.map_to_native_window(clip.origin);
                assert!(
                    origin.y >= at.y - 0.5
                        && origin.y + rect.height() <= at.y + clip.height() + 0.5,
                    "{label} is hidden by its scroll view: {origin:?} {rect:?}, clip {at:?} {clip:?}"
                );
            }
            parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        LogicalPosition::new(origin.x + rect.width() / 2., origin.y + rect.height() / 2.)
    }

    /// Scroll a popup button into its scroll viewports through real wheel input on
    /// each viewport's content margin, then require that as much of it as each
    /// viewport can show is visible. Returns the centre of the visible part.
    fn reveal(&mut self, label: &str) -> LogicalPosition {
        use i_slint_core::{
            item_tree::{ItemRc, ParentItemTraversalMode},
            items::Flickable,
        };
        let vertical = |item: &ItemRc| {
            let rect = item.geometry();
            let origin = item.map_to_native_window(rect.origin);
            (origin.x, origin.y, origin.y + rect.height())
        };
        let item = ui::popup_item(&self.ui.window, label, AccessibleRole::Button);
        let mut viewports = Vec::new();
        let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        while let Some(ancestor) = parent {
            if ancestor.downcast::<Flickable>().is_some() {
                viewports.push(ancestor.clone());
            }
            parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        for viewport in viewports.iter().rev() {
            let (_, top, bottom) = vertical(&item);
            let (left, clip_top, clip_bottom) = vertical(viewport);
            // Align the top edge when the item is taller than the viewport.
            let delta = if top < clip_top {
                clip_top - top
            } else if bottom > clip_bottom {
                (clip_bottom - bottom).max(clip_top - top)
            } else {
                0.
            };
            if delta != 0. {
                self.ui
                    .window
                    .window()
                    .dispatch_event(WindowEvent::PointerScrolled {
                        position: LogicalPosition::new(left + 4., clip_top + 1.),
                        delta_x: 0.,
                        delta_y: delta,
                    });
                self.ui.draw();
            }
        }
        let (x, mut top, mut bottom) = vertical(&item);
        for viewport in &viewports {
            let (_, clip_top, clip_bottom) = vertical(viewport);
            let showable = (bottom - top).min(clip_bottom - clip_top);
            top = top.max(clip_top);
            bottom = bottom.min(clip_bottom);
            assert!(
                bottom - top + 1. >= showable,
                "{label} is not scrolled into its viewport: visible {top}..{bottom}, viewport {clip_top}..{clip_bottom}"
            );
        }
        LogicalPosition::new(x + item.geometry().width() / 2., (top + bottom) / 2.)
    }
}

#[test]
fn popup_acceptance_is_staged_and_cancel_escape_and_x_preserve_the_original_color() {
    let mut h = Harness::new();
    let original = h.ui.window.global::<ColorOptionsView>().get_draft();
    for close in ["Cancel", "Escape", "Cancel dialog"] {
        h.open("Foreground");
        ui::click_popup(&h.ui.window, "RGB 239, 68, 68", AccessibleRole::Button);
        if close == "Escape" {
            ui::key(&h.ui.window, Key::Escape);
        } else {
            ui::click_popup(&h.ui.window, close, AccessibleRole::Button);
        }
        h.ui.draw();
        assert_eq!(h.popups(), 1, "only the chooser closes");
        assert_eq!(
            h.ui.window.global::<ColorOptionsView>().get_draft(),
            original
        );
    }
    h.open("Foreground");
    // Reopening starts from the original value, not the cancelled red preview.
    ui::click_popup(&h.ui.window, "Use color", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        h.ui.window
            .global::<ColorOptionsView>()
            .get_draft()
            .foreground,
        original.foreground
    );
    h.open("Background");
    ui::click_popup(&h.ui.window, "RGB 239, 68, 68", AccessibleRole::Button);
    ui::click_popup(&h.ui.window, "Use color", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        h.ui.window
            .global::<ColorOptionsView>()
            .get_draft()
            .background,
        "#ef4444"
    );
    assert_eq!(h.popups(), 1);
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::SavePreferences { .. }))
    );
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        h.ui.window.global::<ColorOptionsView>().get_draft(),
        original
    );
    assert_eq!(h.popups(), 0);
}

#[test]
fn picker_footer_stays_visible_and_controls_scroll_at_the_minimum_window_size() {
    let mut h = Harness::new();
    h.open("Foreground");
    render::snapshot(&h.ui, "color-picker-popup");
    for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
        h.ui.window.invoke_apply_preferences(1, density);
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(320, 200);
        render::snapshot(&h.ui, &format!("color-picker-resized-{density}-{font}"));
        let accept = h.inside("Use color");
        h.inside("Cancel");
        h.inside("Cancel dialog");
        // Scroll the content margin, not a SpinBox/slider's own wheel handler.
        let position = LogicalPosition::new(16., accept.y - 45.);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.,
                delta_y: -10_000.,
            });
        h.ui.draw();
        // From the end of the form, wheel input brings the palette back into view.
        let red = h.reveal("RGB 239, 68, 68");
        assert!(
            red.y < accept.y,
            "palette scrolls into view above the fixed footer"
        );
        // The palette keeps one whole swatch row beside the fixed footer. Touch
        // density at 32px cannot: its 45px title, two 46px footer rows and a
        // 46px swatch row need 9px more than the 320x200 popup has. This is the
        // known limit in DIALOG_ERGONOMICS.md; drop the exception with its fix.
        if (density, font) != (1, 32) {
            h.inside("RGB 239, 68, 68");
        }
        render::snapshot(&h.ui, &format!("color-picker-small-{density}-{font}"));
    }
    ui::key(&h.ui.window, Key::Escape);
    h.ui.draw();
    assert_eq!(h.popups(), 1);
    // A chooser opened in a tiny window must also stay interactive after growth.
    h.ui.window.invoke_apply_preferences(1, 0);
    h.ui.window.invoke_apply_fonts("".into(), 11, 11);
    h.ui.window.global::<ColorOptionsView>().set_picker_field(0);
    h.ui.draw();
    assert_eq!(h.popups(), 2);
    h.ui.resize(1340, 820);
    h.inside("Use color");
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.popups(), 1, "resized popup must receive pointer input");
}

#[test]
fn changing_options_page_invalidates_the_open_chooser() {
    let mut h = Harness::new();
    let original = h.ui.window.global::<ColorOptionsView>().get_draft();
    h.open("Foreground");
    h.ui.window.set_options_topic_index(0);
    h.ui.draw();
    assert_eq!(h.popups(), 1);
    let colors = h.ui.window.global::<ColorOptionsView>();
    assert_eq!(colors.get_picker_field(), -1);
    colors.invoke_choose_color(0, slint::Color::from_rgb_u8(255, 0, 0));
    assert_eq!(colors.get_draft(), original, "stale choice must be ignored");
}
