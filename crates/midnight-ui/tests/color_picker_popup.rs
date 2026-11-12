//! A color chooser is a separate, scrollable popup within the Options session.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

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

    fn inside(&self, label: &str) -> LogicalPosition {
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
        LogicalPosition::new(origin.x + rect.width() / 2., origin.y + rect.height() / 2.)
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
        let red = h.inside("RGB 239, 68, 68");
        assert!(
            red.y < accept.y,
            "palette scrolls into view above the fixed footer"
        );
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
