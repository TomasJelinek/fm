//! Options must remain usable across ordinary clicks and nested picker popups.
use fm_ui::{MainWindow, Theme};
use slint::{
    ComponentHandle, SharedString,
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

fn surface(window: &MainWindow) -> slint::Rgb8Pixel {
    let color = window.global::<Theme>().get_surface();
    slint::Rgb8Pixel {
        r: color.red(),
        g: color.green(),
        b: color.blue(),
    }
}
fn dialog_top(window: &MainWindow) -> usize {
    (820 - if window.get_density_index() == 1 {
        380
    } else {
        320
    }) / 2
}
fn dialog_visible(window: &MainWindow, pixels: &[slint::Rgb8Pixel]) -> bool {
    // A quiet interior pixel distinguishes the Options surface from the file
    // panels behind it without depending on fonts or text rasterization.
    // Slint positions a popup when shown; a density change can resize its
    // content without moving that anchor. This pixel stays inside both sizes.
    pixels[260 * 1340 + 430] == surface(window)
}
fn control_rows(window: &MainWindow, pixels: &[slint::Rgb8Pixel]) -> Vec<(usize, usize)> {
    // Read painted control edges beyond their text. The pickers are the first
    // two full-height rectangles, independent of font metrics and density.
    let surface = surface(window);
    let top = dialog_top(window);
    let mut rows = Vec::new();
    let mut start = None;
    for y in top + 50..820 - top - 20 {
        if pixels[y * 1340 + 890] != surface {
            start.get_or_insert(y);
        } else if let Some(start) = start.take()
            && y - start >= 24
        {
            rows.push((start, y));
        }
    }
    rows
}

#[test]
fn options_survives_inside_clicks_and_picker_selections() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time))).unwrap();
    let window = MainWindow::new().unwrap();
    fm_ui::install_file_keys(&window);
    window.invoke_apply_preferences(0, 0);
    let accepted = Rc::new(Cell::new((-1, -1)));
    let changed = accepted.clone();
    let weak = window.as_weak();
    window.on_preferences_changed(move |theme, density, _| {
        changed.set((theme, density));
        weak.upgrade()
            .unwrap()
            .invoke_apply_preferences(theme, density);
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
    let open_options = || {
        let theme = window.global::<Theme>();
        let command_y = 820.0
            - theme.get_window_padding()
            - theme.get_bottom_height()
            - theme.get_gap()
            - theme.get_command_height() / 2.0;
        click(&window, 1240.0, command_y);
    };
    open_options();
    draw(&mut pixels);
    assert!(dialog_visible(&window, &pixels), "Options must open");
    click(&window, 470.0, 282.0);
    draw(&mut pixels);
    assert!(
        dialog_visible(&window, &pixels),
        "clicking the Options heading must leave the dialog open"
    );
    let rows = control_rows(&window, &pixels);
    assert!(rows.len() >= 2, "both preference pickers must be visible");
    let theme_row = rows[0];
    click(&window, 650.0, (theme_row.0 + theme_row.1) as f32 / 2.0);
    draw(&mut pixels);
    // The pinned Fluent popup has 40px rows, starting over the combo box.
    click(&window, 650.0, theme_row.0 as f32 + 60.0);
    draw(&mut pixels);
    assert_eq!(accepted.get(), (1, 0), "clicking Dark must apply the theme");
    assert!(
        dialog_visible(&window, &pixels),
        "choosing a theme must keep Options open"
    );
    let density_row = control_rows(&window, &pixels)[1];
    click(&window, 650.0, (density_row.0 + density_row.1) as f32 / 2.0);
    draw(&mut pixels);
    click(&window, 650.0, density_row.0 as f32 + 60.0);
    draw(&mut pixels);
    assert_eq!(
        accepted.get(),
        (1, 1),
        "Touch must apply after choosing a theme"
    );
    assert!(
        dialog_visible(&window, &pixels),
        "changing density must keep Options open"
    );
    let theme_row = control_rows(&window, &pixels)[0];
    click(&window, 650.0, (theme_row.0 + theme_row.1) as f32 / 2.0);
    draw(&mut pixels);
    click(&window, 650.0, theme_row.0 as f32 + 100.0);
    draw(&mut pixels);
    assert_eq!(
        accepted.get(),
        (2, 1),
        "Light must still work after resizing for Touch"
    );
    assert!(dialog_visible(&window, &pixels));
    key(&window, Key::Escape);
    draw(&mut pixels);
    assert!(
        !dialog_visible(&window, &pixels),
        "Escape must close Options"
    );
    open_options();
    draw(&mut pixels);
    assert!(dialog_visible(&window, &pixels));
    // Done is the second button in the bottom row. Find its painted body so
    // text width changes cannot turn this into a click on empty space.
    let y = 820 - dialog_top(&window) - 20 - 22;
    let mut buttons = Vec::new();
    let mut start = None;
    for x in 440..=900 {
        if pixels[y * 1340 + x] != surface(&window) {
            start.get_or_insert(x);
        } else if let Some(start) = start.take()
            && x - start > 25
        {
            buttons.push((start, x));
        }
    }
    assert_eq!(buttons.len(), 2, "Save again and Done must be visible");
    click(
        &window,
        (buttons[1].0 + buttons[1].1) as f32 / 2.0,
        y as f32,
    );
    draw(&mut pixels);
    assert!(!dialog_visible(&window, &pixels), "Done must close Options");
    window.hide().unwrap();
}
