//! Native Slint events exercise permission drafts without touching the filesystem.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable, TextInput},
};
use midnight_ui::MetadataDraft;
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const PERMISSIONS: [&str; 12] = [
    "Owner read",
    "Owner write",
    "Owner execute",
    "Group read",
    "Group write",
    "Group execute",
    "Other read",
    "Other write",
    "Other execute",
    "SUID",
    "SGID",
    "Sticky",
];

struct Harness {
    ui: ui::TestUi,
    drafts: Rc<RefCell<Vec<MetadataDraft>>>,
    dirty: Rc<Cell<usize>>,
}

impl Harness {
    fn new(mode: i32, known: i32) -> Self {
        let mut ui = ui::TestUi::new();
        ui.window.invoke_apply_preferences(1, 0);
        ui.window.set_metadata_can_unix(true);
        ui.window.set_metadata_unix_current_mode(mode);
        ui.window.set_metadata_unix_known_mask(known);
        ui.window.set_metadata_unix_supported_mask(0o7777);
        ui.window
            .set_metadata_summary("Captured Linux permissions".into());
        let drafts = Rc::new(RefCell::new(Vec::new()));
        let captured = drafts.clone();
        ui.window
            .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
        let dirty = Rc::new(Cell::new(0));
        let captured = dirty.clone();
        ui.window
            .on_dirty_metadata(move |_| captured.set(captured.get() + 1));
        let weak = ui.window.as_weak();
        ui.window.on_cancel_metadata(move |_| {
            weak.upgrade().unwrap().set_metadata_token("".into());
        });
        ui.window.set_metadata_token("unix-permissions:1".into());
        settle(&mut ui);
        Self { ui, drafts, dirty }
    }

    fn click(&mut self, label: &str, role: AccessibleRole) {
        ui::click_popup(&self.ui.window, label, role);
        settle(&mut self.ui);
    }

    fn bit(&mut self, label: &str) {
        self.click(label, AccessibleRole::Checkbox);
    }

    fn review(&mut self) -> MetadataDraft {
        let before = self.drafts.borrow().len();
        self.click("Review changes", AccessibleRole::Button);
        let drafts = self.drafts.borrow();
        assert_eq!(
            drafts.len(),
            before + 1,
            "Review must emit the actual draft"
        );
        drafts.last().unwrap().clone()
    }

    fn value(&self, label: &str, property: AccessibleStringProperty) -> String {
        ui::popup_item(&self.ui.window, label, AccessibleRole::Checkbox)
            .accessible_string_property(property)
            .unwrap()
            .to_string()
    }

    fn state(&self, label: &str) -> String {
        self.value(label, AccessibleStringProperty::Value)
    }

    fn octal(&self) -> String {
        let item = ui::popup_item(
            &self.ui.window,
            "Octal permissions",
            AccessibleRole::TextInput,
        );
        text_input_value(item).expect("octal editor contains a native single-line text input")
    }

    fn symbolic(&self, expected: &str) {
        let item = ui::popup_item(&self.ui.window, "Permission text", AccessibleRole::Text);
        let rendered = ui::visible_text_values(item);
        assert!(
            rendered.iter().any(|value| value.as_str() == expected),
            "expected permission text {expected:?}, found {rendered:?}"
        );
    }

    fn type_octal(&mut self, value: &str) {
        self.click("Octal permissions", AccessibleRole::TextInput);
        ui::chord(&self.ui.window, &[Key::Control], "a");
        for letter in value.chars() {
            ui::key(&self.ui.window, letter.to_string());
        }
        settle(&mut self.ui);
        assert_eq!(self.octal(), value, "native typing must retain every digit");
    }

    fn reset(&mut self) {
        self.click("Leave permissions unchanged", AccessibleRole::Button);
    }
}

fn text_input_value(item: ItemRc) -> Option<String> {
    if let Some(input) = item.downcast::<TextInput>() {
        return Some(input.as_pin_ref().text().to_string());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(value) = text_input_value(item) {
            return Some(value);
        }
    }
    None
}

fn settle(test: &mut ui::TestUi) {
    // Embedded popups settle their geometry through deferred property trackers.
    for _ in 0..3 {
        test.draw();
    }
}

fn assert_matrix(draft: &MetadataDraft, mask: i32, value: i32) {
    assert!(!draft.unix_edit, "matrix drafts must leave raw input mode");
    assert!(draft.unix_mode.is_empty());
    assert_eq!((draft.unix_mask, draft.unix_value), (mask, value));
}

fn assert_inside_window(test: &ui::TestUi, label: &str, role: AccessibleRole) {
    let item = ui::popup_item(&test.window, label, role);
    let bounds = item.geometry();
    let origin = item.map_to_native_window(bounds.origin);
    assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
    assert!(
        origin.x >= -0.5
            && origin.y >= -0.5
            && origin.x + bounds.width() <= test.width as f32 + 0.5
            && origin.y + bounds.height() <= test.height as f32 + 0.5,
        "{label} is outside the window: {origin:?} {bounds:?}"
    );
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            let clip = ancestor.map_to_native_window(ancestor.geometry().origin);
            assert!(
                origin.y >= clip.y - 0.5
                    && origin.y + bounds.height() <= clip.y + ancestor.geometry().height() + 0.5,
                "{label} is vertically clipped by its scroll viewport"
            );
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

#[test]
fn current_permissions_are_visible_without_requesting_edits() {
    let mut h = Harness::new(0o600, 0o7777);
    assert_eq!(h.octal(), "600");
    h.symbolic("rw-------");
    for label in PERMISSIONS {
        assert_eq!(
            h.state(label),
            if matches!(label, "Owner read" | "Owner write") {
                "Current: Set"
            } else {
                "Current: Clear"
            }
        );
        assert_inside_window(&h.ui, label, AccessibleRole::Checkbox);
    }
    for title in ["Owner", "Group", "Other", "Read", "Write", "Execute"] {
        let text = ui::popup_item(&h.ui.window, title, AccessibleRole::Text);
        assert!(text.geometry().width() > 0.0 && text.geometry().height() > 0.0);
    }
    assert_inside_window(&h.ui, "Octal permissions", AccessibleRole::TextInput);
    assert_inside_window(&h.ui, "Leave permissions unchanged", AccessibleRole::Button);
    render::snapshot(&h.ui, "unix-permissions");
    assert_eq!(
        h.dirty.get(),
        0,
        "opening must not mark any permission dirty"
    );
    assert_matrix(&h.review(), 0, 0);
}

#[test]
fn individual_permission_edits_preserve_other_bits_and_support_keyboard_reset() {
    let mut h = Harness::new(0o600, 0o7777);
    h.bit("Owner write");
    assert_eq!(h.state("Owner write"), "Change: Clear");
    assert_eq!(h.octal(), "400");
    assert_matrix(&h.review(), 0o200, 0);

    h.bit("Owner write");
    assert_eq!(h.state("Owner write"), "Change: Set");
    assert_matrix(&h.review(), 0o200, 0o200);

    h.bit("Owner write");
    ui::key(&h.ui.window, Key::Delete);
    settle(&mut h.ui);
    assert_eq!(h.state("Owner write"), "Current: Set");
    assert_eq!(h.octal(), "600");
    assert_matrix(&h.review(), 0, 0);

    h.bit("Group read");
    ui::chord(&h.ui.window, &[Key::Shift], Key::Space);
    settle(&mut h.ui);
    assert_eq!(h.state("Group read"), "Current: Clear");
    assert_matrix(&h.review(), 0, 0);

    h.bit("Group read");
    assert_eq!(h.octal(), "640");
    assert_matrix(&h.review(), 0o040, 0o040);
    h.reset();
    assert_matrix(&h.review(), 0, 0);
}

#[test]
fn mixed_permission_bits_are_unknown_until_explicitly_set_or_cleared() {
    let mut h = Harness::new(0o600, 0o7737);
    assert_eq!(h.state("Group read"), "Current: Mixed");
    assert!(h.octal().is_empty());
    h.symbolic("rw-?-----");
    h.bit("Group read");
    assert_eq!(h.state("Group read"), "Change: Set");
    assert_eq!(h.octal(), "640");
    assert_matrix(&h.review(), 0o040, 0o040);
    h.bit("Group read");
    assert_eq!(h.state("Group read"), "Change: Clear");
    assert_eq!(h.octal(), "600");
    assert_matrix(&h.review(), 0o040, 0);
    h.reset();
    assert_eq!(h.state("Group read"), "Current: Mixed");
    assert!(h.octal().is_empty());
    assert_matrix(&h.review(), 0, 0);
}

#[test]
fn native_octal_typing_and_matrix_edits_stay_synchronized() {
    let mut h = Harness::new(0o4600, 0o7777);
    h.type_octal("640");
    assert_eq!(h.state("SUID"), "Current: Set");
    h.symbolic("rwSr-----");
    let draft = h.review();
    assert!(draft.unix_edit);
    assert_eq!(draft.unix_mode, "640");
    assert_eq!((draft.unix_mask, draft.unix_value), (0, 0));

    h.bit("Group write");
    assert_eq!(h.octal(), "4660");
    assert_eq!(h.state("SUID"), "Current: Set");
    h.symbolic("rwSrw----");
    assert_matrix(&h.review(), 0o777, 0o660);

    h.type_octal("2755");
    assert_eq!(h.state("SUID"), "Change: Clear");
    assert_eq!(h.state("SGID"), "Change: Set");
    let draft = h.review();
    assert!(draft.unix_edit);
    assert_eq!(draft.unix_mode, "2755");
    assert_eq!((draft.unix_mask, draft.unix_value), (0, 0));
    h.bit("Other execute");
    assert_eq!(h.octal(), "2754");
    h.symbolic("rwxr-sr--");
    assert_matrix(&h.review(), 0o7777, 0o2754);
}

#[test]
fn invalid_octal_is_retained_and_permission_reset_survives_lost_capabilities() {
    let mut h = Harness::new(0o600, 0o7777);
    h.type_octal("888");
    h.symbolic("Invalid octal permissions");
    assert_eq!(
        h.value("Owner read", AccessibleStringProperty::Enabled),
        "false"
    );
    let before = h.dirty.get();
    h.bit("Owner read");
    assert_eq!(h.octal(), "888");
    assert_eq!(
        h.dirty.get(),
        before,
        "disabled matrix must retain invalid input"
    );
    let draft = h.review();
    assert!(draft.unix_edit);
    assert_eq!(draft.unix_mode, "888");

    h.ui.window.set_metadata_unix_supported_mask(0);
    h.ui.window.set_metadata_can_unix(false);
    settle(&mut h.ui);
    assert_inside_window(&h.ui, "Leave permissions unchanged", AccessibleRole::Button);
    h.reset();
    assert_matrix(&h.review(), 0, 0);

    h.ui.window.set_metadata_unix_supported_mask(0o7777);
    h.ui.window.set_metadata_can_unix(true);
    settle(&mut h.ui);
    assert_eq!(h.octal(), "600");
    h.bit("Owner execute");
    h.ui.window.set_metadata_unix_supported_mask(0);
    h.ui.window.set_metadata_can_unix(false);
    settle(&mut h.ui);
    h.reset();
    assert_matrix(&h.review(), 0, 0);
}

#[test]
fn refreshed_current_permissions_do_not_clobber_the_pending_draft() {
    let mut h = Harness::new(0o600, 0o7777);
    h.bit("Owner execute");
    assert_eq!(h.octal(), "700");
    let edits = h.dirty.get();
    h.ui.window.set_metadata_busy(true);
    h.ui.window.set_metadata_unix_current_mode(0o640);
    h.ui.window.set_metadata_unix_known_mask(0o7777);
    h.ui.window.set_metadata_unix_supported_mask(0o7777);
    h.ui.window.set_metadata_revision("3".into());
    h.ui.window.set_metadata_busy(false);
    settle(&mut h.ui);
    assert_eq!(h.dirty.get(), edits, "refresh must not manufacture edits");
    assert_eq!(h.state("Owner execute"), "Change: Set");
    assert_eq!(h.state("Group read"), "Current: Set");
    assert_eq!(h.octal(), "740");
    assert_matrix(&h.review(), 0o100, 0o100);
    h.reset();
    assert_eq!(h.octal(), "640");
    assert_matrix(&h.review(), 0, 0);
}

#[test]
fn permission_dialog_keeps_dismissal_reachable_at_small_sizes_and_large_fonts() {
    let mut h = Harness::new(0o2755, 0o7777);
    for (index, width, height, font) in [(0, 1340, 960, 32), (1, 320, 200, 32)] {
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(width, height);
        h.ui.window
            .set_metadata_token(format!("unix-layout:{index}").into());
        settle(&mut h.ui);
        assert_inside_window(&h.ui, "Cancel", AccessibleRole::Button);
        assert_inside_window(&h.ui, "Review changes", AccessibleRole::Button);
        // Overflow belongs to the editor viewport; grid rows must still lay out
        // separately rather than shrinking text and hit targets into each other.
        for labels in [
            ["Owner read", "Group read", "Other read"],
            ["Owner write", "Group write", "Other write"],
            ["Owner execute", "Group execute", "Other execute"],
        ] {
            let mut previous_bottom = f32::NEG_INFINITY;
            for label in labels {
                let cell = ui::popup_item(&h.ui.window, label, AccessibleRole::Checkbox);
                let bounds = cell.geometry();
                let top = cell.map_to_native_window(bounds.origin).y;
                assert!(
                    bounds.height() >= font as f32,
                    "{label} lost its readable height"
                );
                assert!(
                    top >= previous_bottom - 0.5,
                    "permission rows overlap at {label}"
                );
                previous_bottom = top + bounds.height();
            }
        }
        render::snapshot(&h.ui, &format!("unix-permissions-layout-{width}-{font}"));
        ui::key(&h.ui.window, Key::Escape);
        settle(&mut h.ui);
        assert!(h.ui.window.get_metadata_token().is_empty());
        assert!(
            !ui::has_popup(&h.ui.window),
            "Escape must close the compact editor"
        );
    }
}
