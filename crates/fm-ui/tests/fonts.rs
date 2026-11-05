//! Font options use real focus and keys, including while enumeration is pending.
#[path = "support/ui.rs"]
mod ui;

use fm_ui::{MainWindow, Theme};
use i_slint_core::items::AccessibleRole;
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
fn key(window: &MainWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
fn click(window: &MainWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}

#[test]
fn font_options_keep_family_and_density_bases_independent() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    fm_ui::install_file_keys(&window);
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
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1340);
        });
    };
    draw(&mut pixels);
    click(
        &window,
        1240.0,
        820.0
            - theme.get_window_padding()
            - theme.get_bottom_height()
            - theme.get_gap()
            - theme.get_command_height() / 2.0,
    );
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
        assert!(theme.get_command_height() >= theme.get_control_height() + 8.0);
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
    window.on_exit(move || captured.set(captured.get() + 1));
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
        ui::click_popup(&window, "Done", AccessibleRole::Button);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert!(
            !ui::has_popup(&window),
            "Done must stay visible at 32px; density={density}"
        );
        window.invoke_focus_files();
        key(&window, Key::F10);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        click(
            &window,
            940.0,
            height as f32 - 75.0 - 12.0 - theme.get_control_height() / 2.0,
        );
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
        click(
            &window,
            520.0,
            height as f32 - 75.0 - 12.0 - theme.get_control_height() / 2.0,
        );
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert_ne!(
            pixels[(height / 2 + 60) * 1040 + 230],
            surface,
            "User menu Close must stay visible at 32px"
        );
    }
    window.hide().unwrap();
}
