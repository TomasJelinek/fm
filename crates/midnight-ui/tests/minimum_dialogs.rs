//! The U-D08 leftovers of the Windows acceptance run 20261121-045727
//! (docs/developer/acceptance/ui-polish.md): at 320×200, in Blue Desktop and Dark Touch,
//! About closes after the gate's walk through its licenses (click the filter, Down,
//! Enter, then Escape), and About, Hot Paths and a notice with its details show no text
//! or control cut off without scrolling (U-K03), no single-line text cut without an
//! ellipsis (U-K04), no zero-size text (U-K18) and no button caption wider than the
//! part of its button that shows (U-K20). Measured with the fonts of the platform the
//! test runs on.
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/owned_commands.rs"]
#[allow(dead_code)] // Only `install` is needed here.
mod owned_commands;
use crate::support::{modeless, ui};

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, Clip, ComplexText, Flickable, SimpleText, TextOverflow, TextWrap},
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use midnight_ui::{AboutView, HotPathsPresentation, NoticePresentation, Presenter};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

/// Blue Desktop and Dark Touch with their default fonts, as the gate captures them.
const LOOKS: [(&str, i32, i32); 2] = [("Blue Desktop", 0, 0), ("Dark Touch", 1, 1)];

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

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
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

fn interactive(role: AccessibleRole) -> bool {
    matches!(
        role,
        AccessibleRole::Button
            | AccessibleRole::Checkbox
            | AccessibleRole::Combobox
            | AccessibleRole::Tab
            | AccessibleRole::TextInput
    )
}

fn needs(item: &ItemRc, orientation: Orientation) -> f32 {
    let adapter = item.window_adapter().expect("a shown item has a window");
    item.borrow()
        .as_ref()
        .layout_info(orientation, -1.0, &adapter, item)
        .preferred
}

/// The texts and captions of one text item: needs no ellipsis, elides, or wraps.
fn text_style(item: &ItemRc) -> (bool, bool) {
    match item.downcast::<ComplexText>() {
        Some(text) => {
            let text = text.as_pin_ref();
            (
                matches!(text.overflow(), TextOverflow::Elide),
                !matches!(text.wrap(), TextWrap::NoWrap),
            )
        }
        None => (false, false),
    }
}

#[derive(Clone, Copy)]
struct Walk {
    /// What clips the item between it and the nearest scroll view: it cuts without
    /// scrolling.
    fixed: Option<Rect>,
    /// Every clipping ancestor: what the user sees of the item.
    shown: Option<Rect>,
    /// The nearest interactive ancestor's visible part (a caption's button).
    button: Option<Rect>,
}

/// What a walk found: the findings, the one-line texts cut by their element (judged
/// once the whole tree is seen: a shortened path label draws a "..." text at its cut
/// edge, which the gate accepts) and the shown "..." and "…" texts.
#[derive(Default)]
struct Found {
    findings: Vec<String>,
    clipped: Vec<(Rect, String, f32)>,
    markers: Vec<Rect>,
}

/// The gate's U-K03, U-K04, U-K18 and U-K20 findings below `root`.
fn findings(root: &ItemRc) -> Vec<String> {
    fn walk(item: &ItemRc, at: Walk, found: &mut Found) {
        if hides(item) {
            return;
        }
        let rect = bounds(item);
        let role = item.accessible_role();
        let shown = at.shown.map_or(rect, |clip| intersect(rect, clip));
        if shown.2 > 0.0
            && shown.3 > 0.0
            && text_of(item).is_some_and(|text| matches!(text.trim(), "..." | "…"))
        {
            found.markers.push(rect);
        }
        let text = text_of(item).filter(|text| text.chars().any(char::is_alphanumeric));
        // A shown text of no size, where what clips it leaves room (the gate's U-K18).
        if let Some(text) = &text
            && (rect.2 <= 0.0 || rect.3 <= 0.0)
            && at.shown.is_none_or(|clip| clip.2 > 0.0 && clip.3 > 0.0)
        {
            let short: String = text.chars().take(40).collect();
            found
                .findings
                .push(format!("{short:?} is {:.0}×{:.0}", rect.2, rect.3));
        }
        if let Some(text) = &text
            && shown.2 > 0.0
            && shown.3 > 0.0
        {
            let short: String = text.chars().take(40).collect();
            let (elides, wraps) = text_style(item);
            if !wraps && !elides && needs(item, Orientation::Horizontal) > rect.2 + 1.0 {
                found
                    .clipped
                    .push((rect, short.clone(), needs(item, Orientation::Horizontal)));
            }
            // A caption against the part of its button that shows.
            if let Some(button) = at.button
                && !wraps
                && !elides
                && needs(item, Orientation::Horizontal) > button.2 + 1.0
            {
                found.findings.push(format!(
                    "caption {short:?} needs {:.0}px, its button shows {:.0}px",
                    needs(item, Orientation::Horizontal),
                    button.2
                ));
            }
        }
        let area = match &text {
            Some(_) => Some((
                rect.0,
                rect.1,
                needs(item, Orientation::Horizontal).min(rect.2),
                rect.3,
            )),
            None if interactive(role) && at.button.is_none() => Some(rect),
            None => None,
        };
        if let (Some(area), Some(fixed)) = (area, at.fixed)
            && area.2 > 0.5
            && area.3 > 0.5
        {
            let outside = (fixed.0 - area.0).max(0.0)
                + (area.0 + area.2 - fixed.0 - fixed.2).max(0.0)
                + (fixed.1 - area.1).max(0.0)
                + (area.1 + area.3 - fixed.1 - fixed.3).max(0.0);
            if outside > 1.5 {
                found.findings.push(format!(
                    "{role:?} {:?} reaches {outside:.0}px outside {fixed:?}, which does not scroll",
                    text.clone().unwrap_or_else(|| label(item))
                ));
            }
        }
        let clips = item.borrow().as_ref().clips_children();
        let next = Walk {
            fixed: if item.downcast::<Flickable>().is_some() {
                None
            } else if clips {
                Some(at.fixed.map_or(rect, |fixed| intersect(fixed, rect)))
            } else {
                at.fixed
            },
            shown: if clips { Some(shown) } else { at.shown },
            button: if role == AccessibleRole::Button {
                Some(shown)
            } else {
                at.button
            },
        };
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            walk(&current, next, found);
        }
    }
    let mut found = Found::default();
    walk(
        root,
        Walk {
            fixed: None,
            shown: None,
            button: None,
        },
        &mut found,
    );
    // A cut text that a shown marker follows at its edge, on its line, is shortened on
    // purpose (the gate's `ellipsis_after`).
    for ((x, y, w, h), short, needed) in &found.clipped {
        let marked = found.markers.iter().any(|marker| {
            (marker.0 - (x + w)).abs() <= 4.0 && marker.1 < y + h && marker.1 + marker.3 > *y
        });
        if !marked {
            found.findings.push(format!(
                "{short:?} needs {needed:.0}px, its element has {w:.0}px and does not elide"
            ));
        }
    }
    found.findings
}

fn dialog_root(window: &slint::Window) -> Option<ItemRc> {
    let inner = WindowInner::from_pub(window);
    let popups = inner.active_popups();
    popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
}

/// The main window with its production callbacks, as About's own tests drive it.
fn about_ui() -> (ui::TestUi, Rc<RefCell<Presenter>>, midnight_ui::BeginOwned) {
    let (mut test, _clock) = ui::TestUi::with_virtual_clock();
    let base = std::env::temp_dir().join("about at the minimum size 文档");
    let presenter = Rc::new(RefCell::new(Presenter::new(
        base.clone(),
        base.join("other"),
    )));
    let begin = owned_commands::install(
        &test.window,
        presenter.clone(),
        Rc::new(|_: midnight_core::Command| true),
    );
    test.window.set_application_version("1.2.3".into());
    test.resize(320, 200);
    (test, presenter, begin)
}

#[test]
fn about_closes_after_the_gates_walk_through_its_licenses_at_the_minimum_size() {
    let (mut test, _presenter, _begin) = about_ui();
    let mut problems = Vec::new();
    for (look, scheme, density) in LOOKS {
        test.window.invoke_apply_fonts("".into(), 12, 15);
        test.window.invoke_apply_preferences(scheme, density);
        test.draw();
        test.window.invoke_show_about();
        test.draw();
        assert!(test.window.get_about_open(), "{look}: About opens");
        ui::click_popup(&test.window, "Licenses", AccessibleRole::Tab);
        test.draw();
        // The gate clicks the filter's centre, presses Down and Enter, then Escape until
        // About closes, at most three times.
        let (x, y, w, h) = bounds(&ui::popup_item(
            &test.window,
            "Filter libraries",
            AccessibleRole::TextInput,
        ));
        ui::click(&test.window, x + w / 2.0, y + h / 2.0);
        test.draw();
        ui::key(&test.window, Key::DownArrow);
        test.draw();
        ui::key(&test.window, Key::Return);
        test.draw();
        if !test.window.global::<AboutView>().get_detail_open() {
            problems.push(format!("{look}: Enter after Down shows no library"));
        }
        for _ in 0..3 {
            if !test.window.get_about_open() {
                break;
            }
            ui::key(&test.window, Key::Escape);
            test.draw();
        }
        if test.window.get_about_open() {
            problems.push(format!("{look}: About is still open after three Escapes"));
            test.window.global::<AboutView>().set_detail_open(false);
            ui::click_popup(&test.window, "Close", AccessibleRole::Button);
            test.draw();
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn about_cuts_nothing_off_at_the_minimum_size() {
    let (mut test, _presenter, _begin) = about_ui();
    let mut problems = Vec::new();
    for (look, scheme, density) in LOOKS {
        test.window.invoke_apply_fonts("".into(), 12, 15);
        test.window.invoke_apply_preferences(scheme, density);
        test.draw();
        test.window.invoke_show_about();
        test.draw();
        let mut check = |test: &ui::TestUi, what: &str| {
            let root = dialog_root(test.window.window()).expect("About is open");
            problems.extend(
                findings(&root)
                    .into_iter()
                    .map(|finding| format!("{look}, {what}: {finding}")),
            );
        };
        check(&test, "About");
        ui::click_popup(&test.window, "Licenses", AccessibleRole::Tab);
        test.draw();
        check(&test, "Licenses");
        // A library's licence, as Enter on the selected one shows it.
        test.window.global::<AboutView>().set_current(0);
        test.window.global::<AboutView>().set_detail_open(true);
        test.draw();
        check(&test, "a library");
        test.window.global::<AboutView>().set_detail_open(false);
        test.draw();
        ui::key(&test.window, Key::Escape);
        test.draw();
        assert!(!test.window.get_about_open(), "{look}: Escape closes About");
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The current folder of the gate's run.
const GATE_FOLDER: &str = "D:\\development\\src\\ME\\.copies\\checkouts\\a-long-working-copy-name-12345\\_build\\acceptance-win\\20261121-045727\\fixture\\review";

#[test]
fn hot_paths_cuts_nothing_off_at_the_minimum_size() {
    let host = form::Host::new();
    let mut problems = Vec::new();
    for (look, scheme, density) in LOOKS {
        host.apply_preferences(scheme, density, "", 12, 15);
        let hot_paths = host.main.global::<HotPathsPresentation>();
        hot_paths.set_path(GATE_FOLDER.into());
        hot_paths.set_path_tail("\\review".into());
        hot_paths.set_token("hot-paths".into());
        host.layout(320, 200);
        let root = host.popup().expect("Hot Paths is open");
        problems.extend(
            findings(&root)
                .into_iter()
                .map(|finding| format!("{look}: {finding}")),
        );
        drop(root);
        hot_paths.set_token("".into());
        host.layout(320, 200);
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn a_notices_details_cut_nothing_off_at_the_minimum_size() {
    let host = form::Host::new();
    let mut problems = Vec::new();
    let path = GATE_FOLDER;
    for (look, scheme, density) in LOOKS {
        host.apply_preferences(scheme, density, "", 12, 15);
        let notice = host.main.global::<NoticePresentation>();
        notice.set_title("Cannot open folder".into());
        notice.set_error(true);
        notice.set_body("Access to this folder was denied.".into());
        notice.set_folder_path(path.into());
        notice.set_details("Error kind: PermissionDenied\nOperation: list folder".into());
        notice.set_generation(notice.get_generation() + 1);
        notice.set_token(format!("notice-{scheme}").into());
        host.layout(320, 200);
        host.click("Details");
        host.layout(320, 200);
        let root = host.popup().expect("the notice is open");
        problems.extend(
            findings(&root)
                .into_iter()
                .map(|finding| format!("{look}: {finding}")),
        );
        drop(root);
        notice.set_token("".into());
        host.layout(320, 200);
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
