//! Real hover, paint and click routing across the shared tooltip compiler adapter.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText, WindowItem},
    window::{PopupWindowLocation, WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, Theme, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const WIDTH: usize = 1340;
const HEIGHT: usize = 900;
struct TestPlatform(
    Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    Rc<Cell<Duration>>,
);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        self.0.borrow_mut().push(native.clone());
        Ok(native)
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}
fn draw(native: &MinimalSoftwareWindow, pixels: &mut [slint::Rgb8Pixel]) {
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(pixels, WIDTH);
    });
}
fn find(item: ItemRc, label: &str, role: AccessibleRole) -> Vec<ItemRc> {
    let mut found = Vec::new();
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value.as_str() == label)
    {
        found.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        found.extend(find(item, label, role));
    }
    found
}
fn root(window: &MainWindow) -> ItemRc {
    ItemRc::new_root(WindowInner::from_pub(window.window()).component())
}
fn popup_root(window: &MainWindow) -> ItemRc {
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("actual dropdown/Options popup");
    ItemRc::new_root(popup.component.clone())
}
fn center(item: &ItemRc) -> slint::LogicalPosition {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    )
}
fn click(window: &MainWindow, position: slint::LogicalPosition) {
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}
fn options_topic(
    window: &MainWindow,
    native: &MinimalSoftwareWindow,
    pixels: &mut [slint::Rgb8Pixel],
    label: &str,
    topic: i32,
) {
    if let Some(button) = ui::try_popup_item(window, label, AccessibleRole::Button) {
        click(window, center(&button));
    } else {
        // Large-font Options uses the compact picker instead of sidebar buttons.
        let current = window.get_options_topic_index();
        let current_index = if current >= 3 { current - 1 } else { current };
        let target_index = if topic >= 3 { topic - 1 } else { topic };
        ui::click_popup(window, "Options topic", AccessibleRole::Combobox);
        for _ in 0..(target_index - current_index).abs() {
            ui::key(
                window,
                if target_index > current_index {
                    Key::DownArrow
                } else {
                    Key::UpArrow
                },
            );
        }
        ui::key(window, Key::Return);
    }
    draw(native, pixels);
    assert_eq!(window.get_options_topic_index(), topic);
}

fn axis(cursor: f32, size: f32, surface: f32, gap: f32) -> f32 {
    let after = cursor + gap;
    (if after + size <= surface - 8.0 {
        after
    } else {
        cursor - gap - size
    })
    .clamp(8.0, (surface - 8.0 - size).max(8.0))
}
fn text_font(item: ItemRc, expected: &str) -> Option<f32> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value.as_str() == expected)
    {
        if let Some(text) = item.downcast::<SimpleText>() {
            return Some(text.as_pin_ref().font_size().get());
        }
        if let Some(text) = item.downcast::<ComplexText>() {
            return Some(text.as_pin_ref().font_size().get());
        }
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(font) = text_font(item, expected) {
            return Some(font);
        }
    }
    None
}
fn start_hover(
    window: &MainWindow,
    native: &MinimalSoftwareWindow,
    pixels: &mut [slint::Rgb8Pixel],
    position: slint::LogicalPosition,
) -> Vec<slint::Rgb8Pixel> {
    window.window().dispatch_event(WindowEvent::PointerExited);
    draw(native, pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    draw(native, pixels);
    pixels.to_vec()
}
fn assert_tip(
    window: &MainWindow,
    pixels: &[slint::Rgb8Pixel],
    quiet: &[slint::Rgb8Pixel],
    position: slint::LogicalPosition,
    expected: &str,
) -> slint::LogicalSize {
    let inner = WindowInner::from_pub(window.window());
    let (origin, component) = {
        let popups = inner.active_popups();
        let tooltip = popups
            .iter()
            .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .unwrap_or_else(|| panic!("timed hover must show tooltip: {expected} at {position:?}"));
        let PopupWindowLocation::ChildWindow(origin) = tooltip.location else {
            panic!("pinned software/Winit tooltip contract is embedded");
        };
        (origin, tooltip.component.clone())
    };
    let item = ItemRc::new_root(component);
    // WindowItem dimensions are assigned by the runtime layout. Its generic
    // item-geometry expression may still contain the compiler's original size.
    let popup_window = item.downcast::<WindowItem>().expect("tooltip Window item");
    let size = slint::LogicalSize::new(
        popup_window.as_pin_ref().width().get(),
        popup_window.as_pin_ref().height().get(),
    );
    assert!(size.width > 0.0 && size.height > 0.0);
    assert!(
        (origin.x - axis(position.x, size.width, WIDTH as f32, 14.0)).abs() < 0.6,
        "horizontal cursor gap/flip: {origin:?} {size:?} {position:?}"
    );
    assert!(
        (origin.y - axis(position.y, size.height, HEIGHT as f32, 18.0)).abs() < 0.6,
        "vertical cursor gap/flip: {origin:?} {size:?} {position:?}"
    );
    assert!(
        origin.x >= 7.9
            && origin.y >= 7.9
            && origin.x + size.width <= WIDTH as f32 - 7.9
            && origin.y + size.height <= HEIGHT as f32 - 7.9
    );
    assert!(position.x < origin.x || position.x >= origin.x + size.width);
    assert!(position.y < origin.y || position.y >= origin.y + size.height);
    assert_eq!(
        text_font(item, expected),
        Some(window.global::<Theme>().get_small_font()),
        "full plain text and smaller themed font must survive clipping: {expected}"
    );
    let mut changed = 0;
    for y in (origin.y.ceil() as usize)..((origin.y + size.height).floor() as usize) {
        for x in (origin.x.ceil() as usize)..((origin.x + size.width).floor() as usize) {
            changed += usize::from(pixels[y * WIDTH + x] != quiet[y * WIDTH + x]);
        }
    }
    assert!(
        changed > 100,
        "actual tooltip content must paint in its bounds"
    );
    size
}
fn hover(
    window: &MainWindow,
    native: &MinimalSoftwareWindow,
    time: &Cell<Duration>,
    pixels: &mut [slint::Rgb8Pixel],
    position: slint::LogicalPosition,
    expected: &str,
) -> slint::LogicalSize {
    let quiet = start_hover(window, native, pixels, position);
    time.set(time.get() + Duration::from_millis(600));
    draw(native, pixels);
    assert_tip(window, pixels, &quiet, position, expected)
}
fn rect(item: &ItemRc) -> (f32, f32, f32, f32) {
    let geometry = item.geometry();
    let origin = item.map_to_native_window(geometry.origin);
    (
        origin.x,
        origin.y,
        origin.x + geometry.size.width,
        origin.y + geometry.size.height,
    )
}
fn moving_footer(
    window: &MainWindow,
    native: &MinimalSoftwareWindow,
    time: &Cell<Duration>,
    pixels: &mut [slint::Rgb8Pixel],
) {
    let hint = "Help (F1)\nApplication window";
    for visible in [false, true] {
        window.window().dispatch_event(WindowEvent::PointerExited);
        window.set_desktop_feedback_important(false);
        window.set_touch_font_size(15);
        window.set_desktop_status("A compact footer moves the existing function row.".into());
        draw(native, pixels);
        let old = rect(&find(root(window), "F1 Help", AccessibleRole::Button).remove(0));
        window.set_desktop_feedback_important(true);
        if visible {
            window.set_touch_font_size(16);
        }
        draw(native, pixels);
        let moved = rect(&find(root(window), "F1 Help", AccessibleRole::Button).remove(0));
        assert!(
            moved.1 < old.1 - 1.0,
            "actual footer must move the hovered button"
        );
        let overlap_top = old.1.max(moved.1);
        let overlap_bottom = old.3.min(moved.3);
        assert!(
            overlap_bottom - overlap_top > 2.0,
            "cursor must remain in both rendered button bounds"
        );
        let position = slint::LogicalPosition::new(
            (old.0.max(moved.0) + old.2.min(moved.2)) / 2.0,
            (overlap_top + overlap_bottom) / 2.0,
        );
        window.set_desktop_feedback_important(false);
        window.set_touch_font_size(15);
        draw(native, pixels);
        let quiet = start_hover(window, native, pixels, position);
        if visible {
            time.set(time.get() + Duration::from_millis(600));
            draw(native, pixels);
            assert_tip(window, pixels, &quiet, position, hint);
        }
        // No pointer replay: layout changes while the same cursor remains inside.
        window.set_desktop_feedback_important(true);
        if visible {
            window.set_touch_font_size(16);
        }
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: "x".into() });
        draw(native, pixels);
        if !visible {
            time.set(time.get() + Duration::from_millis(600));
            draw(native, pixels);
        }
        let actual = rect(&find(root(window), "F1 Help", AccessibleRole::Button).remove(0));
        assert!(
            position.x >= actual.0
                && position.x < actual.2
                && position.y >= actual.1
                && position.y < actual.3
        );
        assert_tip(window, pixels, &quiet, position, hint);
        window.window().dispatch_event(WindowEvent::PointerExited);
        draw(native, pixels);
        assert!(
            WindowInner::from_pub(window.window())
                .active_popups()
                .is_empty(),
            "stock exit must close the final placement after retry"
        );
    }
    window.set_desktop_feedback_important(false);
    window.set_touch_font_size(15);
}
fn navigations(commands: &RefCell<Vec<Command>>) -> usize {
    commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::Navigate { .. } | Command::Refresh { .. }))
        .count()
}

#[test]
fn panel_warning_tooltip_contains_the_reason_and_keeps_its_click_target_clear() {
    let natives = Rc::new(RefCell::new(Vec::new()));
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(natives.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    let native = natives.borrow().last().unwrap().clone();
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.show().unwrap();
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT];
    let reason = "1 unreadable folder: Access denied\nC:\\private\\資料\nClick for full details";
    window.set_left_rows(slint::ModelRc::new(slint::VecModel::from(vec![
        midnight_ui::FileRow {
            identity: "warning:1".into(),
            name: "Folder".into(),
            directory: true,
            size_state: 3,
            size_hint: reason.into(),
            ..Default::default()
        },
    ])));
    draw(&native, &mut pixels);
    let position =
        center(&find(root(&window), "Folder size warning", AccessibleRole::Button).remove(0));
    hover(&window, &native, &time, &mut pixels, position, reason);
    let inspected = Rc::new(Cell::new(false));
    let capture = inspected.clone();
    window.on_panel_warning_details(move |_, _, _| capture.set(true));
    click(&window, position);
    assert!(
        inspected.get(),
        "the tooltip must not cover the warning icon"
    );
    window.hide().unwrap();
}

#[test]
fn shared_tooltips_keep_cursor_clear_in_panels_dropdowns_and_options() {
    let natives = Rc::new(RefCell::new(Vec::new()));
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(natives.clone(), time.clone()))).unwrap();
    for (density, font, scheme) in [
        (UiDensity::Desktop, 11, PreferenceTheme::Blue),
        (UiDensity::Touch, 15, PreferenceTheme::Dark),
        (UiDensity::Desktop, 32, PreferenceTheme::Light),
        (UiDensity::Touch, 32, PreferenceTheme::Blue),
    ] {
        let window = MainWindow::new().unwrap();
        window.set_preferences_ready(true);
        let native = natives.borrow().last().unwrap().clone();
        let path = std::env::temp_dir().join("midnight-tooltip-ui");
        let home = path.join("文档😀".repeat(40));
        let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                density,
                desktop_font_size: font,
                touch_font_size: font,
                theme: scheme,
                ..Default::default()
            }))));
        presenter.borrow_mut().apply(Event::DesktopContext {
            network_position: 0,
            shell: ShellFlavor::Posix,
            locations: vec![NavigationLocation {
                id: "home".into(),
                label: "Home".into(),
                path: home,
                kind: LocationKind::Home,
                drive_letter: None,
            }],
        });
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        refresh(&window, &presenter.borrow());
        window.show().unwrap();
        native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
        });
        let mut pixels = vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT];
        draw(&native, &mut pixels);
        let choice = window
            .get_navigation_choices()
            .iter()
            .find(|row| row.kind == midnight_ui::LocationRole::Home)
            .expect("the fixture exposes a semantic Home location");
        let buttons = find(
            root(&window),
            choice.tooltip.as_str(),
            AccessibleRole::Button,
        );
        assert_eq!(
            buttons.len(),
            2,
            "one actual Home quick button in each pane"
        );
        for button in buttons {
            let position = center(&button);
            hover(
                &window,
                &native,
                &time,
                &mut pixels,
                position,
                choice.tooltip.as_str(),
            );
            let before = navigations(&commands);
            click(&window, position);
            assert_eq!(
                navigations(&commands),
                before + 1,
                "tooltip must pass through quick navigation click"
            );
        }
        for (label, hint) in [
            ("F1 Help", "Help (F1)\nApplication window"),
            ("F10 Menu", "Main menu (F10)\nApplication window"),
        ] {
            let button = find(root(&window), label, AccessibleRole::Button).remove(0);
            hover(&window, &native, &time, &mut pixels, center(&button), hint);
        }
        window.window().dispatch_event(WindowEvent::PointerExited);
        window.set_active_pane(0);
        window.invoke_focus_files();
        ui::chord(&window, &[Key::Control], "g");
        draw(&native, &mut pixels);
        assert!(!find(root(&window), "Folder path", AccessibleRole::TextInput).is_empty());
        let folder_hint = format!(
            "Folder path: {}\n{}{}\nEdit and press Enter to open a folder. Focus path editor (Ctrl+G)",
            window.get_left_path(),
            window.get_left_status(),
            if window.get_left_loading() {
                "\nReading folder…"
            } else {
                ""
            },
        );
        hover(
            &window,
            &native,
            &time,
            &mut pixels,
            // TextInput grows to the whole string; hover its clipped viewport.
            slint::LogicalPosition::new(
                window.get_left_path_left() + window.get_left_path_width() / 2.0,
                window.get_left_path_top() + window.get_left_path_control_height() / 2.0,
            ),
            &folder_hint,
        );
        window.window().dispatch_event(WindowEvent::PointerExited);
        let picker = find(root(&window), "Drive or volume", AccessibleRole::Combobox).remove(0);
        click(&window, center(&picker));
        draw(&native, &mut pixels);
        let row = find(
            popup_root(&window),
            choice.label.as_str(),
            AccessibleRole::Button,
        )
        .remove(0);
        let position = center(&row);
        hover(
            &window,
            &native,
            &time,
            &mut pixels,
            position,
            choice.tooltip.as_str(),
        );
        let before = navigations(&commands);
        click(&window, position);
        assert_eq!(
            navigations(&commands),
            before + 1,
            "nested tooltip must pass through canonical dropdown selection"
        );
        window.window().dispatch_event(WindowEvent::PointerExited);
        let menu = find(root(&window), "F10 Menu", AccessibleRole::Button).remove(0);
        click(&window, center(&menu));
        draw(&native, &mut pixels);
        ui::click_popup(&window, "Menu category", AccessibleRole::Combobox);
        for _ in 0..5 {
            ui::key(&window, Key::DownArrow);
        }
        ui::key(&window, Key::Return);
        draw(&native, &mut pixels);
        let options = find(popup_root(&window), "Options", AccessibleRole::Button).remove(0);
        let position = center(&options);
        hover(&window, &native, &time, &mut pixels, position, "Options");
        click(&window, position);
        draw(&native, &mut pixels);
        options_topic(&window, &native, &mut pixels, "Appearance", 1);
        let theme = find(popup_root(&window), "Theme", AccessibleRole::Combobox).remove(0);
        hover(
            &window,
            &native,
            &time,
            &mut pixels,
            center(&theme),
            "Preview a color theme. OK saves it; Cancel restores the previous theme.",
        );
        window.window().dispatch_event(WindowEvent::PointerExited);
        options_topic(&window, &native, &mut pixels, "General", 0);
        let checkbox = find(
            popup_root(&window),
            "Show only filename search matches",
            AccessibleRole::Checkbox,
        )
        .remove(0);
        let position = center(&checkbox);
        hover(
            &window,
            &native,
            &time,
            &mut pixels,
            position,
            "Typing in a panel searches filename prefixes. Enable this to hide nonmatching files instead of moving among matches.",
        );
        let before = window.get_quick_search_filter();
        click(&window, position);
        assert_ne!(
            window.get_quick_search_filter(),
            before,
            "tooltip must pass through native checkbox input"
        );
        window.window().dispatch_event(WindowEvent::PointerExited);
        let done = find(popup_root(&window), "Cancel", AccessibleRole::Button).remove(0);
        let position = center(&done);
        hover(
            &window,
            &native,
            &time,
            &mut pixels,
            position,
            "Discard all changes, restore previewed appearance and close Options.",
        );
        click(&window, position);
        window.window().dispatch_event(WindowEvent::PointerExited);
        draw(&native, &mut pixels);
        assert!(
            WindowInner::from_pub(window.window())
                .active_popups()
                .is_empty(),
            "Cancel must close through the tooltip"
        );
        if density == UiDensity::Touch && font == 15 {
            moving_footer(&window, &native, &time, &mut pixels);
        }
        window.hide().unwrap();
    }
}

#[test]
fn folder_tab_tooltip_shows_only_the_complete_native_or_archive_path() {
    let natives = Rc::new(RefCell::new(Vec::new()));
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(natives.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    let native = natives.borrow().last().unwrap().clone();
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.show().unwrap();
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT];
    let mut single_line_height = None;
    for (path, wide) in [
        (r"C:\Temp", false),
        (r"D:\文档\bundle.zip\inner.7z\folder", false),
        (
            concat!(
                r"C:\Users\tjelinek\OneDrive - NVIDIA Corporation\Documents\",
                r"Midnight Explorer\Design specifications\Panel navigation and keyboard shortcuts"
            ),
            true,
        ),
    ] {
        window.set_left_tabs(slint::ModelRc::new(slint::VecModel::from(vec![
            midnight_ui::TabRow {
                identity: "tab-1".into(),
                label: "Temp".into(),
                path: path.into(),
                active: true,
            },
        ])));
        draw(&native, &mut pixels);
        let tab = find(
            root(&window),
            &format!("Temp\n{path}"),
            AccessibleRole::Button,
        )
        .remove(0);
        let size = hover(&window, &native, &time, &mut pixels, center(&tab), path);
        let line_height = *single_line_height.get_or_insert(size.height);
        if wide {
            assert!(
                size.width > 560.0,
                "path should use the available width: {size:?}"
            );
            assert!(
                (size.height - line_height).abs() < 1.0,
                "a typical long path should remain on one line: {size:?}"
            );
        }
    }
    window.hide().unwrap();
}
