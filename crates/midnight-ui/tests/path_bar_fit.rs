//! WIN-3 of the Windows acceptance run (docs/developer/acceptance/windows.md): the path
//! bar's components, drawn one caption each, stay inside the bar together with the
//! 32px "Edit folder path" area after them, at every width. Each drawn caption is
//! rounded up to a whole pixel while the bar measures joined captions, which pushed
//! "Edit folder path" up to several pixels out of the bar's clip where a long path
//! just fitted. Measured with the fonts of the platform the test runs on, over a sweep
//! of widths wide enough for wider fonts too.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::PathPartRow;
use slint::{ComponentHandle, ModelRc, VecModel};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn walk(item: &ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(item);
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        walk(&current, found);
    }
}

/// The left panel's "Edit folder path" area and the clip of the bar that holds it.
fn edit_area(test: &ui::TestUi) -> (Rect, Rect) {
    let middle = test.width as f32 / 2.0;
    let mut found = None;
    walk(
        &ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        &mut |item| {
            if found.is_none()
                && item.accessible_role() == AccessibleRole::Button
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|label| label == "Edit folder path")
                && bounds(item).0 < middle
            {
                found = Some(item.clone());
            }
        },
    );
    let area = found.expect("the left path bar's Edit folder path");
    let mut parent = area.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            return (bounds(&area), bounds(&ancestor));
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("the path bar clips its components");
}

/// Path parts as the presenter joins them: each label is the part of its prefix after
/// the previous one.
fn parts(labels: &[&str]) -> (String, Vec<PathPartRow>) {
    let mut prefix = String::new();
    let rows = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            prefix.push_str(label);
            PathPartRow {
                identity: format!("part-{index}").into(),
                label: (*label).into(),
                prefix: prefix.as_str().into(),
            }
        })
        .collect();
    (prefix, rows)
}

#[test]
fn path_components_and_the_edit_area_stay_inside_the_bar_at_every_width() {
    let mut test = ui::TestUi::new();
    // About 120 characters in many short components, as in the run's checkout path:
    // every component adds its own rounding.
    let (path, rows) = parts(&[
        "D:\\",
        "development\\",
        "src\\",
        "ME\\",
        ".copies\\",
        "checkouts\\",
        "a-long-working-copy-name-12345\\",
        "_build\\",
        "acceptance-win\\",
        "20261121-045727\\",
        "fixture\\",
        "left 文档\\",
        "review",
    ]);
    test.window.set_left_path(path.as_str().into());
    test.window
        .set_left_path_parts(ModelRc::new(VecModel::from(rows)));
    let mut findings = Vec::new();
    for (scheme, density, font) in [(2, 0, 12), (0, 0, 12), (1, 1, 15)] {
        test.window.invoke_apply_fonts("".into(), font, font);
        test.window.invoke_apply_preferences(scheme, density);
        // Left panel widths from a quarter of the path to well past its whole width.
        for width in (900..=2400).step_by(3) {
            test.set_size(width, 820);
            test.layout();
            let ((x, _, w, _), (clip_x, _, clip_w, _)) = edit_area(&test);
            let outside = (x + w - (clip_x + clip_w)).max(clip_x - x);
            if outside > 0.5 || w < 31.5 {
                findings.push(format!(
                    "theme {scheme}, density {density}, font {font}, window {width}px: Edit folder path at {x:.0}..{:.0}, {w:.0}px wide, reaches {outside:.1}px outside {clip_x:.0}..{:.0}",
                    x + w,
                    clip_x + clip_w
                ));
            }
        }
    }
    assert!(findings.is_empty(), "{findings:#?}");
}
