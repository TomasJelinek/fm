//! Native renderer regression probe. Run explicitly on a desktop.
#[path = "../tests/support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::Event;
use midnight_ui::{KeyboardEditorView, MainWindow, Presenter};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[path = "native_keyboard_search/external_input.rs"]
mod external_input;
#[path = "native_keyboard_search/fixtures.rs"]
mod fixtures;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .select()?;
    let window = MainWindow::new()?;
    let path = std::env::current_dir()?;
    let (fixture, preferences, compact) = fixtures::load(&path)?;
    eprintln!(
        "native-fixture={fixture}; dynamic-commands={}; density={:?}; desktop-font={}; touch-font={}",
        preferences.user_commands.len(),
        preferences.density,
        preferences.desktop_font_size,
        preferences.touch_font_size
    );
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
    midnight_ui::install(&window, presenter.clone(), Rc::new(|_| true));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    midnight_ui::refresh(&window, &presenter.borrow());
    midnight_ui::install_file_keys(&window);
    window
        .window()
        .set_size(slint::LogicalSize::new(1340., 820.));
    if std::env::args().any(|argument| argument == "--external-input") {
        return external_input::run(window, presenter, compact);
    }
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let window = weak.upgrade().unwrap();
            eprintln!("native search step {step}");
            match step {
                0 => window.invoke_show_options(),
                1 => ui::click_popup(&window, "Keyboard", AccessibleRole::Button),
                2 => ui::click_popup(&window, "Search commands", AccessibleRole::TextInput),
                3..=8 => ui::key(&window, ["c", "o", "p", "y", "x", "z"][step - 3]),
                9..=14 => ui::key(&window, Key::Backspace),
                15 => window
                    .global::<KeyboardEditorView>()
                    .invoke_move_selection(1000000),
                16 => ui::key(&window, "v"),
                17 => ui::key(&window, "i"),
                18 => ui::key(&window, "e"),
                19 => {
                    assert_eq!(window.global::<KeyboardEditorView>().get_filter(), "vie");
                    ui::click_popup(&window, "Done", AccessibleRole::Button);
                }
                20 => window
                    .window()
                    .set_size(slint::LogicalSize::new(320., 200.)),
                21 => {
                    let size = window
                        .window()
                        .size()
                        .to_logical(window.window().scale_factor());
                    assert!(
                        (size.width - 320.).abs() <= 1. && (size.height - 200.).abs() <= 1.,
                        "native minimum is not 320x200: {size:?}"
                    );
                    window
                        .window()
                        .set_size(slint::LogicalSize::new(1340., 820.));
                }
                _ => {
                    let size = window
                        .window()
                        .size()
                        .to_logical(window.window().scale_factor());
                    assert!((size.width - 1340.).abs() <= 1. && (size.height - 820.).abs() <= 1.);
                    eprintln!("Native keyboard search and 320x200 resize/recovery passed");
                    slint::quit_event_loop().unwrap();
                }
            }
            step += 1;
        },
    );
    window.run()?;
    Ok(())
}
