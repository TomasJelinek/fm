//! WIN-2 of the Windows acceptance run (docs/developer/acceptance/windows.md): the
//! command line's long working folder, scrolled to its end, no longer lies under the
//! folder's start drawn over the strip with its "...": the strip is cut where that start
//! ends, so the two texts never overlap (the gate's U-K09), and neither is shown
//! narrower than about a character while it needs more (U-K19), at the sizes and
//! appearances the gate flagged. The strip keeps its place and its scrolling.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip, ComplexText, Flickable, SimpleText},
    layout::Orientation,
    window::WindowInner,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

/// Longer than the prompt shows in every combination below, as the gate's was.
const PATH: &str = "D:\\experiments\\dev\\FM\\.claude\\worktrees\\main-branch-code-review-97667c\\_build\\acceptance-win\\20261121-045727\\work\\ui_polish_dialogs\\fixture\\a rather long folder name\\left 文档";

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

fn text_of(item: &ItemRc) -> Option<(String, f32)> {
    if let Some(text) = item.downcast::<SimpleText>() {
        let text = text.as_pin_ref();
        return Some((text.text().to_string(), text.font_size().get()));
    }
    item.downcast::<ComplexText>().map(|text| {
        let text = text.as_pin_ref();
        (text.text().to_string(), text.font_size().get())
    })
}

fn preferred_width(item: &ItemRc) -> f32 {
    let adapter = item.window_adapter().expect("a shown item has a window");
    item.borrow()
        .as_ref()
        .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
        .preferred
}

/// Hidden by a `visible: false` element on the way up.
fn hidden(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if node.downcast::<Clip>().is_some_and(|clip| {
            let clip = clip.as_pin_ref();
            clip.is_visibility_clip() && clip.clip()
        }) {
            return true;
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

/// What shows of `item`: its bounds cut by every clipping ancestor and the window.
fn shown(item: &ItemRc, window: Rect) -> Rect {
    let mut area = intersect(bounds(item), window);
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            area = intersect(area, bounds(&ancestor));
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    area
}

/// Inside a view that scrolls sideways: its content is wider than the view.
fn in_sideways_scroll(item: &ItemRc) -> bool {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if let Some(view) = ancestor.downcast::<Flickable>()
            && view.as_pin_ref().content_width().get() > bounds(&ancestor).2 + 1.0
        {
            return true;
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

fn walk(item: &ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(item);
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        walk(&current, found);
    }
}

/// The shown texts of the command line's working-folder area, by their shown glyph
/// areas, as the gate measures them (left-aligned glyphs up to their natural width).
fn prompt_texts(test: &ui::TestUi) -> Vec<(String, Rect, f32, f32, bool)> {
    let window = (0.0, 0.0, test.width as f32, test.height as f32);
    let band = (
        0.0,
        test.window.get_command_bar_top(),
        test.width as f32,
        test.window.get_command_bar_height(),
    );
    // The working folder's strip, from its start to the command input.
    let left = test.window.get_command_path_left();
    let right = test.window.get_command_input_left();
    let mut texts = Vec::new();
    walk(
        &ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        &mut |item| {
            let Some((text, font)) = text_of(item) else {
                return;
            };
            if !text.chars().any(char::is_alphanumeric) || hidden(item) {
                return;
            }
            let (x, y, w, h) = bounds(item);
            if y + h / 2.0 < band.1 || y + h / 2.0 > band.1 + band.3 {
                return;
            }
            let glyphs = (x, y, preferred_width(item).min(w), h);
            let area = intersect(glyphs, shown(item, window));
            let middle = area.0 + area.2 / 2.0;
            if area.2 > 0.0 && area.3 > 0.0 && middle >= left - 0.5 && middle <= right + 0.5 {
                let holds = w + 1.0 >= preferred_width(item);
                texts.push((
                    text,
                    area,
                    preferred_width(item),
                    font,
                    holds && in_sideways_scroll(item),
                ));
            }
        },
    );
    texts
}

#[test]
fn the_scrolled_working_folder_never_lies_under_its_shown_start() {
    let mut test = ui::TestUi::new();
    test.window.set_left_path(PATH.into());
    test.window
        .set_terminal_choices(ModelRc::new(VecModel::from(vec![
            SharedString::from("PowerShell"),
            "cmd".into(),
        ])));
    test.window.set_windows_shells(true);
    test.window.set_terminal_available(true);
    test.window.set_terminal_ready(true);
    let mut findings = Vec::new();
    // The gate's flagged combinations, and the Blue Desktop it checked interactively.
    for (scheme, density, width, height) in [
        (1, 1, 960, 600),
        (21, 1, 1340, 820),
        (0, 0, 320, 200),
        (1, 1, 320, 200),
        (0, 0, 1340, 820),
    ] {
        test.window.invoke_apply_fonts("".into(), 12, 15);
        test.window.invoke_apply_preferences(scheme, density);
        test.resize(width, height);
        let case = format!("theme {scheme}, density {density}, {width}×{height}");
        assert!(
            test.window.get_command_path_content_width()
                > test.window.get_command_path_width() + 1.0,
            "{case}: the test needs a shortened working folder"
        );
        let texts = prompt_texts(&test);
        assert!(
            texts
                .iter()
                .any(|(text, ..)| text.starts_with(PATH) && text.ends_with('>')),
            "{case}: the strip shows the prompt's end: {:?}",
            texts.iter().map(|(text, ..)| text).collect::<Vec<_>>()
        );
        for (index, (text, area, needs, font, sideways)) in texts.iter().enumerate() {
            let short: String = text.chars().take(32).collect();
            if *needs > area.2 + 1.0 && area.2 < font.max(8.0) * 1.2 && !sideways {
                findings.push(format!(
                    "{case}: {short:?} shows {:.0}px of {needs:.0}px",
                    area.2
                ));
            }
            for (other, other_area, ..) in &texts[index + 1..] {
                let overlap = intersect(*area, *other_area);
                if overlap.2 > 2.0 && overlap.3 > 2.0 {
                    let other: String = other.chars().take(32).collect();
                    findings.push(format!(
                        "{case}: {short:?} and {other:?} overlap by {:.0}×{:.0}",
                        overlap.2, overlap.3
                    ));
                }
            }
        }
        // The whole folder stays available to assistive technology.
        let label = format!("Working folder: {PATH}");
        assert!(
            ui::try_root_item(&test.window, &label, AccessibleRole::Text).is_some_and(|item| {
                item.accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|value| value == label.as_str())
            }),
            "{case}: the working folder's accessible name"
        );
    }
    assert!(findings.is_empty(), "{findings:#?}");
}
