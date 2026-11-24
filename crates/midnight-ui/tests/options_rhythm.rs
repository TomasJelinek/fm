//! The Options pages' rhythm (the owner's request of 24 November: shorter pages, check
//! boxes closer together): options stacked in a column keep one gap, the shared
//! `OptionRows.row-gap` (U-K39 of the polish gate), while each check box row keeps the
//! control height as its hit target and its focus ring. Prints every page's content
//! height, in both densities, for the record (`--nocapture`).
#[path = "support/window_polish.rs"]
mod polish;
use crate::support::ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::Flickable,
    window::{WindowInner, WindowKind},
};
use slint::ComponentHandle;

const TOPICS: [(i32, &str); 10] = [
    (0, "General"),
    (1, "Appearance"),
    (8, "Colors"),
    (6, "Previews"),
    (7, "Archives"),
    (4, "Editors"),
    (10, "Comparators"),
    (9, "Custom Commands"),
    (3, "Keyboard"),
    (5, "Plugins"),
];

fn flickables(item: ItemRc, found: &mut Vec<ItemRc>) {
    if item.downcast::<Flickable>().is_some() {
        found.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        flickables(current, found);
    }
}

/// The Options dialog's popup root.
fn options_root(window: &slint::Window) -> ItemRc {
    WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
        .next_back()
        .expect("the Options dialog")
}

/// The page's content height: from the top of the page's viewport (the largest Flickable
/// of the dialog, beside the topic list and above the footer) to the bottom of its lowest
/// control or text, scrolled into view or not.
fn page_height(root: &ItemRc) -> f32 {
    let mut found = Vec::new();
    flickables(root.clone(), &mut found);
    let Some(page) = found.iter().max_by(|a, b| {
        let area = |item: &ItemRc| item.geometry().width() * item.geometry().height();
        area(a).total_cmp(&area(b))
    }) else {
        return 0.0;
    };
    let origin = page.map_to_native_window(page.geometry().origin);
    let size = page.geometry().size;
    let elements = polish::alignment_elements(root);
    // The page's viewport: the one of the first element shown inside it.
    let Some(scope) = elements
        .iter()
        .find(|element| {
            element.scope != 0
                && element.rect.0 >= origin.x - 0.5
                && element.rect.0 + element.rect.2 <= origin.x + size.width + 0.5
                && element.rect.1 >= origin.y - 0.5
                && element.rect.1 + element.rect.3 <= origin.y + size.height + 0.5
        })
        .map(|element| element.scope)
    else {
        return 0.0;
    };
    elements
        .iter()
        .filter(|element| element.scope == scope)
        .map(|element| element.rect.1 + element.rect.3 - origin.y)
        .fold(0.0, f32::max)
}
#[test]
fn option_rows_keep_one_gap_on_every_page() {
    let mut ui = ui::TestUi::new();
    let mut findings = Vec::new();
    let mut heights = Vec::new();
    for density in [0, 1] {
        ui.window.invoke_apply_preferences(0, density);
        ui.resize(1340, 820);
        ui.window.invoke_show_options();
        for (topic, name) in TOPICS {
            ui.window.set_options_topic_index(topic);
            ui.draw();
            let root = options_root(ui.window.window());
            heights.push(format!(
                "{name} ({}): {:.0}px",
                if density == 0 { "desktop" } else { "touch" },
                page_height(&root)
            ));
            findings.extend(
                polish::alignment::check(&polish::alignment_elements(&root), None)
                    .into_iter()
                    .filter(|finding| finding.kind == polish::alignment::OPTION_ROWS)
                    .map(|finding| format!("{name}, density {density}: {finding}")),
            );
            // Every check box row stays a full control high: its hit target.
            let control = ui
                .window
                .global::<midnight_ui::Theme>()
                .get_control_height();
            for element in polish::alignment_elements(&root) {
                if element.role == polish::alignment::Role::Toggle
                    && element.whole
                    && element.rect.3 + 0.5 < control
                {
                    findings.push(format!(
                        "{name}, density {density}: {} is lower than a control ({control}px)",
                        element.name
                    ));
                }
            }
        }
        ui.window.set_options_open(false);
        ui.draw();
    }
    eprintln!("Options page heights:\n{}", heights.join("\n"));
    ui.window.hide().unwrap();
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn general_page_check_boxes_follow_each_other_at_the_shared_gap() {
    let mut ui = ui::TestUi::new();
    for density in [0, 1] {
        ui.window.invoke_apply_preferences(0, density);
        ui.resize(1340, 820);
        ui.window.invoke_show_options();
        ui.window.set_options_topic_index(0);
        ui.draw();
        let root = options_root(ui.window.window());
        let theme = ui.window.global::<midnight_ui::Theme>();
        let mut boxes: Vec<_> = polish::alignment_elements(&root)
            .into_iter()
            .filter(|element| element.role == polish::alignment::Role::Toggle && element.whole)
            .map(|element| element.rect)
            .collect();
        boxes.sort_by(|a, b| a.1.total_cmp(&b.1));
        // The Panels group's three check boxes, first on the page.
        let gaps: Vec<f32> = boxes
            .windows(2)
            .take(2)
            .map(|pair| pair[1].1 - (pair[0].1 + pair[0].3))
            .collect();
        let wanted = if density == 0 { 2.0 } else { 4.0 };
        assert!(
            gaps.iter().all(|gap| (gap - wanted).abs() <= 0.5),
            "density {density}: gaps {gaps:?}, wanted {wanted}px; rows {boxes:?}, control {}",
            theme.get_control_height()
        );
        ui.window.set_options_open(false);
        ui.draw();
    }
    ui.window.hide().unwrap();
}
