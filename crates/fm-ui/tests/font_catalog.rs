//! A large installed-font catalog must stay lazy and commit only accepted choices.
#[path = "support/ui.rs"]
mod ui;

use fm_ui::MainWindow;
use i_slint_core::items::AccessibleRole;
use slint::{
    ComponentHandle, Model, ModelRc, ModelTracker, SharedString,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
struct Catalog(Rc<Cell<usize>>);
impl Model for Catalog {
    type Data = SharedString;
    fn row_count(&self) -> usize {
        16_384
    }
    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.0.set(self.0.get() + 1);
        (row < self.row_count()).then(|| format!("Family {row:05} 文档").into())
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &()
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
fn installed_fonts_virtualize_and_keep_the_host_selection_authoritative() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    fm_ui::install_file_keys(&window);
    let reads = Rc::new(Cell::new(0));
    window.set_font_choices(ModelRc::new(Catalog(reads.clone())));
    window.set_font_list_ready(true);
    window.set_font_index(7000);
    let selected = Rc::new(Cell::new(-1));
    let captured = selected.clone();
    window.on_font_options_changed(move |index, _, _, _| captured.set(index));
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
    click(&window, 1240.0, 771.0);
    draw(&mut pixels);
    window.set_options_topic_index(1);
    draw(&mut pixels);
    // Theme establishes focus; Tab follows the Appearance form.
    ui::click_popup(&window, "Theme", AccessibleRole::Combobox);
    key(&window, Key::Escape);
    key(&window, Key::Tab);
    key(&window, Key::Tab);
    key(&window, Key::Return);
    draw(&mut pixels);
    assert!(
        reads.get() < 1024,
        "opening a font catalog eagerly read {} rows",
        reads.get()
    );
    key(&window, Key::End);
    draw(&mut pixels);
    assert_eq!(
        selected.get(),
        -1,
        "keyboard browsing must not commit fonts"
    );
    assert!(reads.get() < 2048, "jumping to End must stay virtualized");
    key(&window, Key::Return);
    assert_eq!(selected.get(), 16_383);
    assert_eq!(
        window.get_font_index(),
        7000,
        "the host may reject a family choice"
    );
    key(&window, Key::Return);
    draw(&mut pixels);
    key(&window, Key::Return);
    assert_eq!(
        selected.get(),
        7000,
        "reopening must follow the host's accepted index"
    );
    selected.set(-1);
    key(&window, Key::Return);
    draw(&mut pixels);
    key(&window, Key::Home);
    window.set_font_choices(ModelRc::new(Catalog(reads.clone())));
    draw(&mut pixels);
    key(&window, Key::Escape);
    assert_eq!(
        selected.get(),
        -1,
        "catalog replacement must cancel pending selection"
    );
    window.hide().unwrap();
}
