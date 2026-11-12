//! Only actual folder-size failures receive an error glyph in each panel view.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{ComplexText, SimpleText},
    window::WindowInner,
};
use midnight_ui::{FileRow, Theme};
use slint::{Brush, ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;

fn error_glyphs(item: ItemRc) -> usize {
    let mut count = usize::from(
        item.accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|label| label == "Folder size warning"),
    );
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        count += error_glyphs(item);
    }
    count
}

#[test]
fn actual_size_errors_have_one_glyph_and_unfinished_sizes_have_none_in_every_view() {
    let mut test = ui::TestUi::new();
    test.window.set_left_rows(ModelRc::new(VecModel::from(
        [
            ("Failed folder", 3),
            ("Running folder", 1),
            ("Cancelled folder", 2),
        ]
        .into_iter()
        .map(|(name, state)| FileRow {
            identity: name.into(),
            name: name.into(),
            directory: true,
            size: "≈ 4 KB".into(),
            size_state: state,
            size_running: state == 1,
            ..Default::default()
        })
        .collect::<Vec<_>>(),
    )));
    for mode in 0..3 {
        test.window.set_left_view_mode(mode);
        slint::platform::update_timers_and_animations();
        test.draw();
        let tree = WindowInner::from_pub(test.window.window()).component();
        assert_eq!(
            error_glyphs(ItemRc::new_root(tree)),
            1,
            "mode {mode}: failed estimates get a glyph; running/cancelled estimates do not"
        );
        render::snapshot(&test, &format!("folder-size-error-mode-{mode}"));
    }
    test.window.hide().unwrap();
}

fn visible_text_style(item: ItemRc, value: &str) -> Option<(Brush, i32)> {
    if item.is_visible() {
        if let Some(text) = item.downcast::<SimpleText>() {
            let text = text.as_pin_ref();
            if text.text() == value {
                return Some((text.color(), text.font_weight()));
            }
        }
        if let Some(text) = item.downcast::<ComplexText>() {
            let text = text.as_pin_ref();
            if text.text() == value {
                return Some((text.color(), text.font_weight()));
            }
        }
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(style) = visible_text_style(item, value) {
            return Some(style);
        }
    }
    None
}

#[test]
fn terminal_size_warnings_keep_normal_ink_while_running_warnings_are_highlighted() {
    let mut test = ui::TestUi::new();
    let rows = Rc::new(VecModel::from(vec![
        FileRow {
            identity: "completed-warning".into(),
            name: "Partially counted folder".into(),
            directory: true,
            size: "≈ 4 KB".into(),
            size_state: 3,
            size_running: false,
            size_hint: "Access denied in a child folder".into(),
            ..Default::default()
        },
        FileRow {
            identity: "running-warning".into(),
            name: "Still counting with a failed child".into(),
            directory: true,
            size: "≈ 8 KB".into(),
            size_state: 3,
            size_running: true,
            size_hint: "Access denied in a child folder; counting continues".into(),
            ..Default::default()
        },
        FileRow {
            identity: "completed-size".into(),
            name: "Counted folder".into(),
            directory: true,
            size: "16 KB".into(),
            ..Default::default()
        },
    ]));
    test.window.set_left_rows(rows.clone().into());
    test.window.set_left_view_mode(0);
    for scheme in [
        midnight_core::PreferenceTheme::Dark,
        midnight_core::PreferenceTheme::Light,
    ] {
        test.window.global::<Theme>().set_scheme(scheme.index());
        let mut running = rows.row_data(1).unwrap();
        running.size_running = true;
        rows.set_row_data(1, running);
        test.draw();
        let tree = WindowInner::from_pub(test.window.window()).component();
        let theme = test.window.global::<Theme>();
        let normal = (Brush::SolidColor(theme.get_secondary_text()), 400);
        let highlighted = (Brush::SolidColor(theme.get_pending_size()), 600);
        assert_ne!(
            normal.0, highlighted.0,
            "{scheme:?} must distinguish active size ink"
        );
        assert_eq!(
            visible_text_style(ItemRc::new_root(tree.clone()), "16 KB"),
            Some(normal.clone())
        );
        assert_eq!(
            visible_text_style(ItemRc::new_root(tree.clone()), "≈ 4 KB"),
            Some(normal.clone()),
            "{scheme:?}: a completed partial result keeps ordinary size ink and weight"
        );
        assert_eq!(
            visible_text_style(ItemRc::new_root(tree.clone()), "≈ 8 KB"),
            Some(highlighted),
            "{scheme:?}: an active partial result stays highlighted despite warning severity"
        );
        assert_eq!(error_glyphs(ItemRc::new_root(tree)), 2);

        let mut completed = rows.row_data(1).unwrap();
        completed.size_running = false;
        rows.set_row_data(1, completed);
        test.draw();
        let tree = WindowInner::from_pub(test.window.window()).component();
        assert_eq!(
            visible_text_style(ItemRc::new_root(tree.clone()), "≈ 8 KB"),
            Some(normal),
            "{scheme:?}: finishing removes progress styling without losing the partial total"
        );
        assert_eq!(
            error_glyphs(ItemRc::new_root(tree)),
            2,
            "finishing must retain both warnings"
        );
    }
    test.window.hide().unwrap();
}
