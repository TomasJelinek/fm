//! Exercise compact controls through actual pointer and keyboard dispatch.
use fm_ui::{LocationRow, MainWindow, Theme};
use slint::{
    ComponentHandle, Model, ModelRc, SharedString, VecModel,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
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

fn toolbar_y(window: &MainWindow) -> f32 {
    let theme = window.global::<Theme>();
    theme.get_window_padding()
        + 1.0
        + 3.0
        + if window.get_left_tabs().row_count() > 0 {
            theme.get_control_height()
        } else {
            0.0
        }
        + theme.get_toolbar_height() / 2.0
}

fn location_strip_x(window: &MainWindow) -> f32 {
    let theme = window.global::<Theme>();
    theme.get_window_padding()
        + 1.0
        + if theme.get_density() == 1 {
            6.0 + 88.0 + 6.0
        } else {
            4.0 + 78.0 + 4.0
        }
}

#[test]
fn icon_shell_picker_commits_only_on_enter_and_restores_editor_focus() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.set_terminal_available(true);
    window.set_terminal_choices(ModelRc::new(VecModel::from(vec![
        "PowerShell — PowerShell syntax".into(),
        "Command Prompt — cmd.exe syntax".into(),
    ])));
    window.set_terminal_index(0);
    let picked = Rc::new(Cell::new(-1));
    let selected = picked.clone();
    let allow_pick = Rc::new(Cell::new(true));
    let allowed = allow_pick.clone();
    let weak = window.as_weak();
    window.on_terminal_changed(move |index| {
        selected.set(index);
        if allowed.get() {
            weak.upgrade().unwrap().set_terminal_index(index);
        }
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
    window.set_left_locations(ModelRc::new(VecModel::from(vec![
        LocationRow {
            label: "Home folder with a long descriptive caption".into(),
            path: "/home/example".into(),
            tooltip: "Home folder with a long descriptive caption\n/home/example".into(),
            ..Default::default()
        },
        LocationRow {
            label: "D:\\".into(),
            compact_label: "D:".into(),
            is_drive: true,
            path: "D:\\".into(),
            tooltip: "D:\\ — Work 文档 · 420 GiB free".into(),
            ..Default::default()
        },
    ])));
    let location_picked = Rc::new(Cell::new(-1));
    let location_selected = location_picked.clone();
    window.on_location(move |pane, index| {
        assert_eq!(pane, 0);
        location_selected.set(index);
    });
    draw(&mut pixels);
    let theme = window.global::<Theme>();
    click(
        &window,
        location_strip_x(&window) + theme.get_tool_width() + 2.0 + 23.0,
        toolbar_y(&window),
    );
    assert_eq!(
        location_picked.get(),
        1,
        "the compact drive target must sit next to the volume dropdown"
    );
    for density in [0, 1] {
        window.invoke_apply_preferences(0, density);
        draw(&mut pixels);
        let theme = window.global::<Theme>();
        if density == 1 {
            click(
                &window,
                location_strip_x(&window) + theme.get_tool_width() / 2.0,
                toolbar_y(&window),
            );
            assert_eq!(
                location_picked.get(),
                0,
                "Touch keeps a compact Home target beside the volume dropdown"
            );
            draw(&mut pixels);
        }
        let x = theme.get_window_padding() + if density == 1 { 30.0 } else { 26.0 };
        let y = 820.0
            - theme.get_window_padding()
            - theme.get_bottom_height()
            - theme.get_gap()
            - theme.get_command_height() / 2.0;
        click(&window, x, y);
        draw(&mut pixels);
        key(&window, Key::DownArrow);
        assert_eq!(picked.get(), -1, "browsing the popup must not change shell");
        key(&window, Key::Return);
        assert_eq!(
            picked.get(),
            1,
            "the far-left icon must open the shell picker"
        );
        assert_eq!(window.get_terminal_index(), 1);
        assert!(window.get_command_focused());
        key(&window, "echo 文档");
        assert_eq!(window.get_command_text().as_str(), "echo 文档");
        window.invoke_set_command("".into(), 0);
        draw(&mut pixels);
        click(&window, x, y);
        draw(&mut pixels);
        key(&window, Key::Home);
        key(&window, Key::Escape);
        assert_eq!(
            window.get_terminal_index(),
            1,
            "Escape cancels the pending choice"
        );
        // A host rejection, such as a cross-shell draft, must remain the
        // selection the picker shows when opened again.
        allow_pick.set(false);
        click(&window, x, y);
        draw(&mut pixels);
        key(&window, Key::Home);
        key(&window, Key::Return);
        assert_eq!(picked.get(), 0);
        assert_eq!(window.get_terminal_index(), 1);
        click(&window, x, y);
        draw(&mut pixels);
        key(&window, Key::Return);
        assert_eq!(picked.get(), 1, "reopening must follow the accepted shell");
        allow_pick.set(true);
        picked.set(-1);
        window.set_terminal_index(0);
        draw(&mut pixels);
    }
    window.invoke_apply_preferences(0, 0);
    window.set_volumes(ModelRc::new(VecModel::from(vec![
        "C:\\ — System · 121 GiB free".into(),
        "D:\\ — Work 文档 · 420 GiB free".into(),
    ])));
    window.set_volume_short_labels(ModelRc::new(VecModel::from(vec![
        "C:\\".into(),
        "D:\\".into(),
    ])));
    window.set_left_volume(0);
    let volume_picked = Rc::new(Cell::new(-1));
    let volume_selected = volume_picked.clone();
    window.on_volume(move |pane, index| {
        assert_eq!(pane, 0);
        volume_selected.set(index);
    });
    draw(&mut pixels);
    let theme = window.global::<Theme>();
    let toolbar_top = theme.get_window_padding() + 1.0 + 3.0 + theme.get_locations_height();
    let drive_x = theme.get_window_padding() + 1.0 + 4.0 + 20.0;
    let drive_y = toolbar_top + theme.get_toolbar_height() / 2.0;
    click(&window, drive_x, drive_y);
    draw(&mut pixels);
    key(&window, Key::DownArrow);
    assert_eq!(volume_picked.get(), -1);
    key(&window, Key::Return);
    assert_eq!(
        volume_picked.get(),
        1,
        "compact drive menu must support keyboard selection"
    );
    volume_picked.set(-1);
    draw(&mut pixels);
    click(&window, drive_x, drive_y);
    draw(&mut pixels);
    let popup_row_y = toolbar_top
        + (theme.get_toolbar_height() + theme.get_control_height()) / 2.0
        + 4.0
        + theme.get_control_height() / 2.0;
    let position = slint::LogicalPosition::new(drive_x, popup_row_y);
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window.set_volumes(ModelRc::new(VecModel::from(vec![
        "D:\\ — Replacement drive".into(),
        "C:\\ — System".into(),
    ])));
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(
        volume_picked.get(),
        -1,
        "replacing drives during a press must cancel selection"
    );
    // Display strings can remain equal when distinct raw mount identities are
    // reordered. The host revision must also invalidate the pressed row.
    click(&window, drive_x, drive_y);
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window.set_volumes_revision(1);
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(
        volume_picked.get(),
        -1,
        "an identity revision must cancel a press even when all display labels are equal"
    );
    draw(&mut pixels);
    let bottom_y = 820.0 - theme.get_window_padding() - theme.get_bottom_height() / 2.0;
    let f2_x = theme.get_window_padding() + (1340.0 - 2.0 * theme.get_window_padding()) * 0.15;
    let position = slint::LogicalPosition::new(f2_x, bottom_y);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    draw(&mut pixels);
    let before_tip = pixels.clone();
    time.set(Duration::from_millis(1000));
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    let changed = before_tip
        .iter()
        .zip(&pixels)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 100,
        "a disabled F2 control must show a real hover tooltip"
    );
    let with_tip = pixels.clone();
    click(&window, f2_x, bottom_y);
    assert_eq!(
        picked.get(),
        -1,
        "a disabled function must never select a shell"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(0.0, 0.0),
    });
    draw(&mut pixels);
    assert!(
        pixels != with_tip,
        "leaving the control must dismiss its tooltip"
    );
    // Many drives stay inside a flexible inline strip rather than pushing
    // essential tools outside a narrow pane, including in the Touch profile.
    window.set_left_locations(ModelRc::new(VecModel::from(
        (0..32)
            .map(|index| LocationRow {
                label: format!("Very long drive caption 文档 {index}").into(),
                compact_label: format!("{index}:").into(),
                is_drive: true,
                tooltip: format!("Native drive {index} · full name and capacity").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    let action = Rc::new(Cell::new((-1, -1)));
    let captured = action.clone();
    window.on_action(move |pane, selected| captured.set((pane, selected)));
    native.set_size(slint::PhysicalSize::new(1040, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1040.0, 820.0),
    });
    for density in [0, 1] {
        window.invoke_apply_preferences(0, density);
        window.invoke_focus_files();
        draw(&mut pixels);
        let theme = window.global::<Theme>();
        assert_eq!(
            theme.get_locations_height(),
            0.0,
            "no extra drive row remains"
        );
        let pane_width = (1040.0 - 2.0 * theme.get_window_padding() - theme.get_gap()) / 2.0;
        let toolbar_padding = if density == 1 { 6.0 } else { 4.0 };
        let toolbar_gap = if density == 1 { 6.0 } else { 4.0 };
        let star_x = theme.get_window_padding() + pane_width
            - 1.0
            - toolbar_padding
            - theme.get_tool_width() / 2.0;
        let refresh_x = star_x - theme.get_tool_width() - toolbar_gap;
        click(&window, refresh_x, toolbar_y(&window));
        assert_eq!(action.get(), (0, 3), "drives cannot obscure Refresh");
        click(&window, star_x, toolbar_y(&window));
        draw(&mut pixels);
        assert_eq!(window.get_focused_pane(), -1, "Bookmarks stays reachable");
        key(&window, Key::Escape);
        action.set((-1, -1));
    }
    window.hide().unwrap();
}
