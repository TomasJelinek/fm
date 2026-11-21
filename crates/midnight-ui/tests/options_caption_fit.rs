//! Polish batch 3, the U-D08 leftover (docs/developer/acceptance/ui-polish.md): at
//! 320×200 with large fonts, the Options pages cut no single-line caption without an
//! ellipsis ("Show commands containing", "No Custom Commands configured.", "Thumbnail
//! size (longest side)", "No matching built-in file types" were cut at 20–24 px) and no
//! wrapped caption short of its lines. Measured with the fonts of the platform the test
//! runs on.
use crate::support::ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{Clip, ComplexText, SimpleText, TextOverflow, TextWrap},
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use midnight_ui::ColorOptionsView;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, platform::Key};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn text_of(item: &ItemRc) -> Option<String> {
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    item.downcast::<ComplexText>()
        .map(|text| text.as_pin_ref().text().to_string())
}

/// A `visible: false` element: Slint lowers it to a clip that hides its subtree.
fn hides(item: &ItemRc) -> bool {
    item.downcast::<Clip>().is_some_and(|clip| {
        let clip = clip.as_pin_ref();
        clip.is_visibility_clip() && clip.clip()
    })
}

/// Every item below `root` that no `visible: false` element hides, in tree order.
fn shown_items(root: &ItemRc) -> Vec<ItemRc> {
    fn walk(item: &ItemRc, found: &mut Vec<ItemRc>) {
        if hides(item) {
            return;
        }
        found.push(item.clone());
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            walk(&current, found);
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found
}

fn needs(item: &ItemRc, orientation: Orientation) -> f32 {
    let adapter = item.window_adapter().expect("a shown item has a window");
    item.borrow()
        .as_ref()
        .layout_info(orientation, -1.0, &adapter, item)
        .preferred
}

fn dialog_root(window: &slint::Window) -> ItemRc {
    let inner = WindowInner::from_pub(window);
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("Options is open");
    ItemRc::new_root(popup.component.clone())
}

/// Shown texts cut short, as the polish gate measures them: a single-line text that
/// needs more width than it has and neither elides nor ends at an omission marker
/// (U-K04), one squeezed to less than about a character (U-K18, U-K19), and a wrapped
/// text with less height than its lines need (U-K05).
fn texts_cut_short(root: &ItemRc) -> Vec<String> {
    let items = shown_items(root);
    let mut found = Vec::new();
    for item in &items {
        let Some(text) = text_of(item) else { continue };
        if !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        let (elides, wraps, size) = match item.downcast::<ComplexText>() {
            Some(complex) => {
                let complex = complex.as_pin_ref();
                (
                    matches!(complex.overflow(), TextOverflow::Elide),
                    !matches!(complex.wrap(), TextWrap::NoWrap),
                    complex.font_size().get(),
                )
            }
            None => (
                false,
                false,
                item.downcast::<SimpleText>()
                    .map_or(12.0, |simple| simple.as_pin_ref().font_size().get()),
            ),
        };
        let (x, y, w, h) = bounds(item);
        let short: String = text.chars().take(40).collect();
        if w <= 0.0 || h <= 0.0 {
            found.push(format!("{short:?} is {w:.0}×{h:.0}"));
            continue;
        }
        if wraps {
            let lines = needs(item, Orientation::Vertical);
            if lines > h + 1.0 {
                found.push(format!(
                    "{short:?} wraps to {lines:.0}px of lines in {h:.0}px"
                ));
            }
            continue;
        }
        let needed = needs(item, Orientation::Horizontal);
        if needed <= w + 1.0 {
            continue;
        }
        if w < size.max(8.0) * 1.2 {
            found.push(format!("{short:?} shows {w:.0} of {needed:.0}px"));
            continue;
        }
        let marked = items.iter().any(|other| {
            text_of(other).is_some_and(|text| matches!(text.trim(), "…" | "..."))
                && (bounds(other).0 - (x + w)).abs() <= 4.0
                && bounds(other).1 < y + h
                && bounds(other).1 + bounds(other).3 > y
        });
        if !elides && !marked {
            found.push(format!(
                "{short:?} needs {needed:.0}px, has {w:.0}px and no ellipsis"
            ));
        }
    }
    found
}

#[test]
fn options_captions_stay_whole_at_the_minimum_size_with_large_fonts() {
    // No Presenter: its refresh would apply its own saved theme and fonts again. The
    // pages show their empty states (no custom commands, no matching file types); a
    // filename colour rule is chosen, so that the Colors page shows its editor.
    let mut test = ui::TestUi::new();
    let colors = test.window.global::<ColorOptionsView>();
    colors.set_labels(ModelRc::new(VecModel::from(vec![SharedString::from(
        "*.rs;*.toml · 文档",
    )])));
    colors.set_selected(0);
    let mut findings = Vec::new();
    for (scheme, density) in [(0, 0), (1, 1)] {
        for font in [16, 18, 20, 22, 24] {
            test.window.invoke_apply_fonts("".into(), font, font);
            test.window.invoke_apply_preferences(scheme, density);
            test.resize(320, 200);
            test.window.invoke_show_options();
            for topic in [0, 1, 3, 4, 10, 5, 6, 7, 8, 9] {
                test.window.set_options_topic_index(topic);
                test.draw();
                let root = dialog_root(test.window.window());
                for finding in texts_cut_short(&root) {
                    findings.push(format!(
                        "density {density}, font {font}, topic {topic}: {finding}"
                    ));
                }
            }
            ui::key(&test.window, Key::Escape);
            test.draw();
        }
    }
    assert!(findings.is_empty(), "cut at 320×200: {findings:#?}");
}
