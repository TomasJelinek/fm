//! Only actual folder-size failures receive an error glyph in each panel view.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, window::WindowInner,
};
use midnight_ui::FileRow;
use slint::{ComponentHandle, ModelRc, VecModel};

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
