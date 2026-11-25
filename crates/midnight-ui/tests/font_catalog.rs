//! A large installed-font catalog must stay lazy and commit only accepted choices.
use crate::support::ui;

use i_slint_core::items::AccessibleRole;
use slint::{ComponentHandle, Model, ModelRc, ModelTracker, SharedString, platform::Key};
use std::{cell::Cell, rc::Rc};

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
// The shared helper sends keys the way the platform does (on Windows, with the
// Winit-shaped key metadata).
use ui::key;
#[test]
fn installed_fonts_virtualize_and_keep_the_host_selection_authoritative() {
    // The shared host represents loaded preferences; Options otherwise
    // intentionally blocks edits until the asynchronous initial load completes.
    let test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let native = test.native.clone();
    let reads = Rc::new(Cell::new(0));
    window.set_font_choices(ModelRc::new(Catalog(reads.clone())));
    window.set_font_list_ready(true);
    window.set_font_index(7000);
    let selected = Rc::new(Cell::new(-1));
    let captured = selected.clone();
    window.on_font_options_changed(move |index, _, _, _| captured.set(index));
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
    window.set_options_topic_index(1);
    draw(&mut pixels);
    // Theme establishes focus; Tab follows the Appearance form: Application icon,
    // Spacing, then the font family.
    ui::click_popup(&window, "Theme", AccessibleRole::Combobox);
    key(&window, Key::Escape);
    key(&window, Key::Tab);
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
