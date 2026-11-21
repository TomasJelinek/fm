//! WIN-7 of the Windows acceptance run (the find record's F-L04): a Find window under
//! 360 px high with text over 20 px shows its idle status in the query area, as FIND.md
//! says, and keeps it in view there: on the area's last line, below the scrolled query
//! form, instead of at the end of that form, scrolled out of view, where the user did
//! not see how the search ended. The query's mask field stays in view above it.
use crate::support::modeless;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
};
use midnight_ui::FindWindow;
use slint::{ComponentHandle, platform::software_renderer::MinimalSoftwareWindow};
use std::{cell::RefCell, rc::Rc};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

const STATUS: &str = "Completed · 46 found · 46 examined";

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

/// How many pixels of `item` its clipping ancestors (scroll views among them) and the
/// window cut off, top and bottom.
fn cut_off(item: &ItemRc, height: f32) -> f32 {
    let (_, top, _, h) = bounds(item);
    let (mut shown_top, mut shown_bottom) = (top.max(0.0), (top + h).min(height));
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            let (_, clip_top, _, clip_h) = bounds(&ancestor);
            shown_top = shown_top.max(clip_top);
            shown_bottom = shown_bottom.min(clip_top + clip_h);
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    h - (shown_bottom - shown_top).max(0.0)
}

fn open() -> (FindWindow, Rc<MinimalSoftwareWindow>) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    (window, native)
}

#[test]
fn a_short_idle_find_with_large_text_keeps_its_status_in_view() {
    let (window, native) = open();
    window.set_progress(STATUS.into());
    let mut findings = Vec::new();
    for (scheme, density, font) in [(1, 1, 21), (1, 1, 24), (0, 0, 22), (0, 0, 32)] {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        for (width, height) in [(700, 340), (480, 300), (320, 200)] {
            modeless::draw(&native, width, height, "find-compact-idle-status");
            let case = format!("theme {scheme}, density {density}, font {font}, {width}×{height}");
            match modeless::accessible_item(&native, AccessibleRole::Button, STATUS) {
                None => findings.push(format!("{case}: no status line")),
                Some(line) => {
                    let cut = cut_off(&line, height as f32);
                    if cut > 0.5 {
                        findings.push(format!(
                            "{case}: the status line shows {:.0} of {:.0}px",
                            bounds(&line).3 - cut,
                            bounds(&line).3
                        ));
                    }
                }
            }
            // The mask stays reachable in the query area above the status.
            let masks =
                modeless::accessible_item(&native, AccessibleRole::TextInput, "File name masks")
                    .expect("the masks field");
            if cut_off(&masks, height as f32) >= bounds(&masks).3 - 0.5 {
                findings.push(format!("{case}: the masks field is out of view"));
            }
        }
    }
    window.hide().unwrap();
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn the_compact_status_line_still_opens_the_details() {
    let (window, native) = open();
    window.invoke_apply_preferences(1, 1, "".into(), 24, 24);
    window.set_progress(STATUS.into());
    window.set_details("46 files · 3 folders".into());
    modeless::draw(&native, 700, 340, "find-compact-status-details");
    let line = modeless::accessible_item(&native, AccessibleRole::Button, STATUS)
        .expect("the status line");
    modeless::click_item(&native, &line, STATUS);
    assert!(
        window.get_show_details(),
        "the status line opens the details"
    );
    window.hide().unwrap();
}
