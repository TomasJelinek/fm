//! Font options use real focus and keys, including while enumeration is pending.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{MainWindow, Theme};
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

// The shared helpers send keys the way the platform does (on Windows, with the
// Winit-shaped key metadata).
use ui::{click, key};

#[test]
fn font_options_keep_family_and_density_bases_independent() {
    let test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let native = test.native.clone();
    let theme = window.global::<Theme>();
    window.invoke_apply_fonts("".into(), 11, 15);
    assert_eq!(theme.get_font_family().as_str(), "");
    assert_eq!(theme.get_font_size(), 11.0);
    assert_eq!(theme.get_small_font(), 10.0);
    assert_eq!(theme.get_hint_font(), 10.0);
    assert_eq!(theme.get_row_height(), 22.0);
    assert_eq!(theme.get_bottom_height(), 22.0);
    let families = vec![
        SharedString::from("Application default"),
        "文档 字体 Cafe\u{301}".into(),
        "Noto Sans 👩🏽‍💻".into(),
    ];
    window.set_font_choices(ModelRc::new(VecModel::from(families.clone())));
    window.set_font_index(1);
    window.invoke_apply_fonts(families[1].clone(), 11, 15);
    let changes = Rc::new(RefCell::new(vec![]));
    let captured = changes.clone();
    let weak = window.as_weak();
    window.on_font_options_changed(move |index, desktop, touch, field| {
        captured.borrow_mut().push((index, desktop, touch, field));
        let window = weak.upgrade().unwrap();
        window.set_font_index(index);
        window.invoke_apply_fonts(
            if index == 0 {
                "".into()
            } else {
                families[index as usize].clone()
            },
            desktop,
            touch,
        );
    });
    let opened = Rc::new(Cell::new(0));
    let captured = opened.clone();
    window.on_options_opened(move || captured.set(captured.get() + 1));
    let launches = Rc::new(Cell::new(0));
    let captured = launches.clone();
    window.on_submit_command(move |_| {
        captured.set(captured.get() + 1);
        false
    });
    // Partial repaints build on the shared host's last full frame.
    let mut pixels = test.pixels.clone();
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1340);
        });
    };
    draw(&mut pixels);
    ui::click_root(&window, "Options", AccessibleRole::Button);
    draw(&mut pixels);
    assert_eq!(opened.get(), 1, "opening Options must request fonts lazily");
    let surface = theme.get_surface();
    let surface = slint::Rgb8Pixel {
        r: surface.red(),
        g: surface.green(),
        b: surface.blue(),
    };
    window.set_options_topic_index(1);
    draw(&mut pixels);
    ui::click_popup(&window, "Theme", AccessibleRole::Combobox);
    draw(&mut pixels);
    key(&window, Key::Escape);
    key(&window, Key::Tab); // Density.
    key(&window, Key::Tab); // Family is disabled while loading; desktop remains usable.
    key(&window, Key::DownArrow);
    assert_eq!(
        changes.borrow().last(),
        Some(&(1, 12, 15, 1)),
        "changing size before enumeration must preserve the configured Unicode family"
    );
    assert_eq!(theme.get_font_family().as_str(), "文档 字体 Cafe\u{301}");
    assert_eq!(theme.get_font_size(), 12.0);
    assert_eq!(theme.get_small_font(), 11.0);
    key(&window, Key::Tab);
    key(&window, Key::DownArrow);
    assert_eq!(changes.borrow().last(), Some(&(1, 12, 16, 2)));
    window.set_font_list_ready(true);
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    key(&window, Key::Tab);
    key(&window, Key::Tab);
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    key(&window, Key::DownArrow);
    key(&window, Key::DownArrow);
    key(&window, Key::Return);
    assert_eq!(
        changes.borrow().last(),
        Some(&(2, 12, 16, 0)),
        "the actual family picker must emit the chosen family index"
    );
    assert_eq!(theme.get_font_family().as_str(), "Noto Sans 👩🏽‍💻");
    assert_eq!(
        launches.get(),
        0,
        "changing fonts must never execute a command"
    );
    key(&window, Key::Escape);
    window.invoke_apply_preferences(0, 1);
    assert_eq!(theme.get_font_size(), 16.0);
    assert_eq!(theme.get_small_font(), 14.0);
    assert_eq!(theme.get_hint_font(), 13.0);
    assert!(theme.get_control_height() >= 40.0);
    window.invoke_apply_fonts("".into(), 32, 32);
    for density in 0..2 {
        window.invoke_apply_preferences(0, density);
        assert_eq!(theme.get_font_size(), 32.0);
        assert!(theme.get_row_height() >= 40.0);
        assert!(theme.get_control_height() >= 42.0);
        assert_eq!(theme.get_command_height(), theme.get_bottom_height());
    }
    window.invoke_apply_fonts("".into(), 8, 8);
    assert_eq!(theme.get_small_font(), 8.0);
    assert_eq!(theme.get_hint_font(), 8.0);
    native.set_size(slint::PhysicalSize::new(1040, 480));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1040.0, 480.0),
    });
    pixels.resize(1040 * 480, slint::Rgb8Pixel::default());
    let exits = Rc::new(Cell::new(0));
    let captured = exits.clone();
    window.on_function_action(move |action, _| {
        if action == "app.exit" {
            captured.set(captured.get() + 1);
        }
    });
    window.invoke_apply_fonts("".into(), 32, 32);
    for density in 0..2 {
        window.invoke_apply_preferences(0, density);
        let height = window.get_effective_minimum_height().ceil().max(480.0) as usize;
        native.set_size(slint::PhysicalSize::new(1040, height as u32));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(1040.0, height as f32),
        });
        pixels.resize(1040 * height, slint::Rgb8Pixel::default());
        window.invoke_focus_files();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        ui::click_root(&window, "Options", AccessibleRole::Button);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert!(
            ui::has_popup(&window),
            "Options opens at 32px; density={density}"
        );
        assert!(window.get_options_left() >= 0.0 && window.get_options_top() >= 0.0);
        assert!(window.get_options_left() + window.get_options_width() <= 1040.0);
        assert!(window.get_options_top() + window.get_options_height() <= height as f32);
        ui::click_popup(&window, "Cancel", AccessibleRole::Button);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert!(
            !ui::has_popup(&window),
            "Cancel must stay visible at 32px; density={density}"
        );
        window.invoke_focus_files();
        key(&window, Key::F10);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        ui::click_popup(&window, "Menu category", AccessibleRole::Combobox);
        for _ in 0..5 {
            ui::key(&window, Key::DownArrow);
        }
        ui::key(&window, Key::Return);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        ui::click_popup(&window, "Exit", AccessibleRole::Button);
        assert_eq!(
            exits.get(),
            density + 1,
            "the final menu action must remain visible at 32px"
        );
        key(&window, Key::Escape);
        window.invoke_focus_files();
        key(&window, Key::F9);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert_eq!(pixels[(height / 2 + 60) * 1040 + 230], surface);
        // The User menu is sized for all of its rows, so its last row, Close, is
        // visible at 32px without scrolling. Close must close the menu itself: any
        // click outside, or on another row, would also close the popup.
        let rows_run = Rc::new(Cell::new(0));
        let bump = |counter: &Rc<Cell<i32>>| {
            let counter = counter.clone();
            move |_: i32| counter.set(counter.get() + 1)
        };
        window.on_properties(bump(&rows_run));
        window.on_folder_sizes(bump(&rows_run));
        window.on_calculate_checksums(bump(&rows_run));
        let (x, y) = unscrolled(&window, "Close", 1040.0, height as f32).center;
        click(&window, x, y);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert!(
            !ui::has_popup(&window),
            "User menu Close must be visible without scrolling at 32px and close the menu; density={density}"
        );
        assert_eq!(rows_run.get(), 0, "Close must not run another menu row");
        assert_ne!(
            pixels[(height / 2 + 60) * 1040 + 230],
            surface,
            "the closed menu no longer covers the panels"
        );
    }
    window.hide().unwrap();
}

/// The User menu is sized for its title and all of its rows at every font size and
/// density: Close, the last row, is visible without scrolling, and less than a row
/// of the list stays blank below it.
#[test]
fn user_menu_fits_its_rows_at_every_font_size() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    midnight_ui::install_file_keys(&window);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1040, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1040.0, 820.0),
    });
    for (density, font) in [(0, 12), (0, 13), (0, 16), (0, 32), (1, 15), (1, 32)] {
        window.invoke_apply_fonts("".into(), font, font);
        window.invoke_apply_preferences(0, density);
        window.invoke_focus_files();
        key(&window, Key::F9);
        ui::settle_layout(window.window());
        let close = unscrolled(&window, "Close", 1040.0, 820.0);
        assert!(
            close.below < close.height,
            "the User menu leaves a blank row below Close at {font}px, density {density}: {}px below a {}px row",
            close.below,
            close.height
        );
        key(&window, Key::Escape);
        ui::settle_layout(window.window());
        assert!(
            !ui::has_popup(&window),
            "Escape closes the User menu; font={font}, density={density}"
        );
    }
    window.hide().unwrap();
}

/// An open popup's button, asserted to be inside the window and every scrolling
/// ancestor's clip without scrolling anything (a clipped row could only be hit
/// through another one).
struct Unscrolled {
    center: (f32, f32),
    height: f32,
    /// How far the innermost scrolling clip extends below the button.
    below: f32,
}

fn unscrolled(window: &MainWindow, label: &str, width: f32, height: f32) -> Unscrolled {
    use i_slint_core::{
        item_tree::{ItemRc, ParentItemTraversalMode},
        items::Flickable,
    };
    let bounds = |item: &ItemRc| {
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        (
            origin.x,
            origin.y,
            origin.x + rect.width(),
            origin.y + rect.height(),
        )
    };
    let item = ui::popup_item(window, label, AccessibleRole::Button);
    let area = bounds(&item);
    assert!(
        area.0 >= 0.0 && area.1 >= 0.0 && area.2 <= width && area.3 <= height,
        "{label} outside the window: {area:?}"
    );
    let mut below = None;
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            let clip = bounds(&ancestor);
            assert!(
                area.0 >= clip.0 - 0.5
                    && area.1 >= clip.1 - 0.5
                    && area.2 <= clip.2 + 0.5
                    && area.3 <= clip.3 + 0.5,
                "{label} needs scrolling: {area:?}, clip {clip:?}"
            );
            if below.is_none() {
                below = Some(clip.3 - area.3);
            }
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    Unscrolled {
        center: ((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0),
        height: area.3 - area.1,
        below: below.expect("the button is inside a scrolling list"),
    }
}
