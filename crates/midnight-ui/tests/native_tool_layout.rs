//! Tool bodies retain geometry, artwork and keyboard dismissal coverage.
//! Backend identity and window-owner semantics live in the installed adapter tests.
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/window_input.rs"]
mod input;
use crate::support::modeless;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, Flickable, SimpleText},
    window::WindowInner,
};
use midnight_ui::{
    BookmarkRow, DecisionWindow, HotPathsPresentation, JobsView, JobsWindow, OperationRow,
};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn platform() -> Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    windows
}
fn appearances() -> [(i32, i32, i32); 4] {
    use midnight_core::PreferenceTheme;
    [
        (PreferenceTheme::Light.index(), 0, 11),
        (PreferenceTheme::Dark.index(), 1, 15),
        (PreferenceTheme::HighContrastDark.index(), 0, 32),
        (PreferenceTheme::HighContrastLight.index(), 1, 32),
    ]
}
fn marker() -> slint::Image {
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(16, 16);
    pixels.make_mut_slice().fill(slint::Rgb8Pixel {
        r: 241,
        g: 17,
        b: 199,
    });
    slint::Image::from_rgb8(pixels)
}
fn render(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
    name: &str,
) -> Vec<slint::Rgb8Pixel> {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
    }
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    window.window().request_redraw();
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    }));
    modeless::bitmap::snapshot(&pixels, width as usize, height as usize, name);
    pixels
}
fn visible_controls(item: ItemRc, label: &str, found: &mut Vec<ItemRc>) {
    // Native titlebars suppress the embedded header, whose close button can
    // share the client footer's label. Validate the actual visible controls.
    if item.accessible_role() == AccessibleRole::Button
        && item.is_visible()
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        found.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        visible_controls(item, label, found);
    }
}
fn visible(window: &MinimalSoftwareWindow, label: &str) -> Vec<ItemRc> {
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    let mut found = Vec::new();
    visible_controls(root, label, &mut found);
    found
}
fn inside(window: &MinimalSoftwareWindow, label: &str) -> ItemRc {
    let item = visible(window, label)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("Missing visible native control: {label}"));
    check_inside(window, &item, label);
    item
}
/// Each of the `count` visible controls that share `label`, such as a header close
/// and a footer button, checked as [`inside`] checks one.
fn inside_each(window: &MinimalSoftwareWindow, label: &str, count: usize) {
    let controls = visible(window, label);
    assert_eq!(controls.len(), count, "visible {label} controls");
    for item in &controls {
        check_inside(window, item, label);
    }
}
fn check_inside(window: &MinimalSoftwareWindow, item: &ItemRc, label: &str) {
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    let size = window.window().size();
    assert!(
        item.is_visible()
            && rect.size.width > 0.0
            && rect.size.height > 0.0
            && at.x >= 0.0
            && at.y >= 0.0
            && at.x + rect.size.width <= size.width as f32 + 1.0
            && at.y + rect.size.height <= size.height as f32 + 1.0,
        "{label} clipped in {size:?}: {at:?}, {rect:?}"
    );
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            let clip = ancestor.geometry();
            let origin = ancestor.map_to_native_window(clip.origin);
            assert!(
                at.x >= origin.x - 1.0
                    && at.y >= origin.y - 1.0
                    && at.x + rect.size.width <= origin.x + clip.size.width + 1.0
                    && at.y + rect.size.height <= origin.y + clip.size.height + 1.0,
                "{label} is inside the window but hidden by a scroll viewport"
            );
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}
fn painted_icon(pixels: &[slint::Rgb8Pixel], name: &str) {
    let count = pixels
        .iter()
        // Icons are dimmed into the theme, so match the marker's magenta hue, not its exact value.
        .filter(|pixel| {
            i16::from(pixel.r) > i16::from(pixel.g) + 40
                && i16::from(pixel.b) > i16::from(pixel.g) + 40
        })
        .count();
    assert!(
        count >= 64,
        "{name} lost its native artwork: {count} marker pixels"
    );
}
fn find_text(item: ItemRc, value: &str) -> Option<ItemRc> {
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if text.is_some_and(|text| text == value) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_text(item, value) {
            return Some(found);
        }
    }
    None
}

#[test]
fn native_jobs_keeps_normal_action_heights_artwork_and_compact_close() {
    let windows = platform();
    let window = JobsWindow::new().unwrap();
    window.global::<JobsView>().on_find(|rows, identity| {
        rows.iter()
            .position(|row| row.identity == identity)
            .map_or(-1, |index| index as i32)
    });
    window.set_rows(slint::ModelRc::new(slint::VecModel::from(vec![OperationRow {
        identity: "copy-job:1".into(), title: "Copy · 文档 · waiting for a decision".into(),
        detail: "123 456 bytes copied · 123 456 bytes verified with SHA-256\n/very/long/文档/source-file.txt".into(),
        cancellable: true, attention: true, icon: marker(), ..Default::default()
    }])));
    window.set_requested_identity("copy-job:1".into());
    let weak = window.as_weak();
    window.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for (scheme, density, font) in appearances() {
        window.show().unwrap();
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        let pixels = render(
            &native,
            1040,
            720,
            &format!("native-jobs-{scheme}-{density}-{font}"),
        );
        for label in ["Cancel job", "Resolve", "Close"] {
            assert!(
                inside(&native, label).geometry().height() <= 100.0,
                "Job actions must not stretch to fill the viewport"
            );
        }
        painted_icon(&pixels, "Jobs row");
        render(&native, 320, 200, "native-jobs-compact");
        inside(&native, "Close");
        input::key(&native, Key::Escape);
        assert!(
            !window.window().is_visible(),
            "compact Jobs Escape must dismiss at scheme {scheme}, density {density}, font {font}"
        );
    }
}

#[test]
fn native_decision_keeps_recovery_details_artwork_and_reachable_footer() {
    let windows = platform();
    let window = DecisionWindow::new().unwrap();
    window.set_token("copy:3:7".into());
    window.set_title_text("Name conflict".into());
    window.set_initial_name("資料.txt".into());
    window.set_allow_alternate_name(true);
    window.set_native_icon(marker());
    let source = "C:\\Users\\Example\\資料.txt\n\nFile\n123 456 bytes (120.6 KiB)\nModified: 07.11.2026 14:22:11\n2026-11-07 13:22:11.123456700 UTC";
    let target = "D:\\Work\\資料.txt\n\nFile\n234 567 bytes (229.1 KiB)\nModified: 06.11.2026 18:10:01\n2026-11-06 17:10:01.500 UTC";
    window.set_source_details(source.into());
    window.set_target_details(target.into());
    window.set_body("Replacement keeps the original in a private recovery folder. Recent Jobs show its location; copy details before closing the app.".into());
    let dismissed = Rc::new(Cell::new(0));
    let observed = dismissed.clone();
    let weak = window.as_weak();
    window.on_request_dismiss(move || {
        observed.set(observed.get() + 1);
        weak.upgrade().unwrap().hide().unwrap();
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for (scheme, density, font) in appearances() {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        window.set_can_replace(false);
        // The recovery detail block has its own scrolling; check all actions
        // in the representative no-details state, then restore full details.
        window.set_source_details("".into());
        let pixels = render(
            &native,
            1040,
            720,
            &format!("native-decision-{scheme}-{density}-{font}"),
        );
        for label in ["Skip", "Skip all", "Copy as", "Cancel job"] {
            inside(&native, label);
        }
        // The header close defers like the footer button and shares its label.
        inside_each(&native, "Decide later", 2);
        painted_icon(&pixels, "Decision body");
        window.set_source_details(source.into());
        window.set_can_replace(true);
        for (width, height) in [(1040, 720), (660, 500), (320, 200)] {
            render(&native, width, height, "native-decision-recovery");
            inside_each(&native, "Decide later", 2);
            let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
            for text in [source, target] {
                assert!(
                    find_text(root.clone(), text).is_some(),
                    "full recovery comparison text was lost"
                );
            }
        }
    }
    input::key(&native, Key::Escape);
    assert_eq!(
        dismissed.get(),
        1,
        "Escape uses the window's own dismissal, which defers like its X"
    );
    assert!(!window.window().is_visible());
}

/// The embedded Hot Paths manager with one saved path, opened; Close and Escape
/// dismiss it as the adapter does.
fn hot_paths_host() -> form::Host {
    let host = form::Host::new();
    let presentation = host.main.global::<HotPathsPresentation>();
    presentation.set_path("/very/long/文档/👩🏽‍💻/folder".into());
    presentation.set_model(slint::ModelRc::new(slint::VecModel::from(vec![
        BookmarkRow {
            identity: "1".into(),
            label: "文档 · Saved project".into(),
            path: "/very/long/文档/👩🏽‍💻/project".into(),
            slot: 1,
        },
    ])));
    presentation.set_action_icons(midnight_ui::ActionIcons {
        add_bookmark: marker(),
        ..Default::default()
    });
    let weak = host.main.as_weak();
    presentation.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<HotPathsPresentation>().set_token("".into());
        }
    });
    presentation.set_token("hot-paths".into());
    host
}

#[test]
fn embedded_hot_paths_keeps_actions_artwork_and_compact_close() {
    let host = hot_paths_host();
    for (scheme, density, font) in appearances() {
        host.apply_preferences(scheme, density, "", font, font);
        host.layout(1040, 720);
        // As tall as the main window allows, like the 1040×720 manager window this
        // test used to render; the default height is checked separately below.
        host.stretch_vertically(&host.item(AccessibleRole::Button, "Close"));
        let pixels = host.render(
            1040,
            720,
            &format!("embedded-hot-paths-{scheme}-{density}-{font}"),
        );
        for label in ["Add current", "Close"] {
            form::assert_unclipped(&host.visible_item(AccessibleRole::Button, label), label);
        }
        painted_icon(&pixels, "Add Hot Path");
        host.render(328, 208, "embedded-hot-paths-compact");
        form::assert_unclipped(&host.visible_item(AccessibleRole::Button, "Close"), "Close");
    }
    input::key(&host.native, Key::Escape);
    assert!(
        !host.is_open(),
        "compact Hot Paths Escape must dismiss the embedded manager"
    );
}

/// At its default height in a roomy main window the manager shows Add current
/// without scrolling, including the wrapped hint and the two-row shortcut picker of
/// a 32 px font: the dialog's size follows HotPathsDialog's natural height.
#[test]
fn embedded_hot_paths_default_height_shows_add_current_at_every_font() {
    let host = hot_paths_host();
    for (scheme, density, font) in appearances() {
        host.apply_preferences(scheme, density, "", font, font);
        host.layout(1040, 720);
        for label in ["Add current", "Close"] {
            form::assert_unclipped(
                &host.visible_item(AccessibleRole::Button, label),
                &format!("{label} at a {font} px font"),
            );
        }
    }
}
