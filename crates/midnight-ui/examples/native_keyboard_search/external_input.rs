//! Native message driver handshake. All preferences remain in the probe's presenter.
use super::{AccessibleRole, ComponentHandle, Duration, KeyboardEditorView, MainWindow, ui};
use midnight_ui::Presenter;
use slint::Model;
use std::{cell::RefCell, rc::Rc};

pub(super) fn run(
    window: MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    compact: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    let mut previous = None;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(20),
        move || {
            let window = weak.upgrade().unwrap();
            match step {
                0 => window.invoke_show_options(),
                1 => ui::click_popup(&window, "Keyboard", AccessibleRole::Button),
                2 => {
                    window
                        .global::<KeyboardEditorView>()
                        .invoke_move_selection(1000000);
                    ui::click_popup(&window, "Search commands", AccessibleRole::TextInput);
                    if compact {
                        window
                            .window()
                            .set_size(slint::LogicalSize::new(320., 200.));
                    }
                }
                3 => {
                    let center = |label| {
                        let item = ui::popup_item(&window, label, AccessibleRole::TextInput);
                        let rect = item.geometry();
                        let origin = item.map_to_native_window(rect.origin);
                        let size = window
                            .window()
                            .size()
                            .to_logical(window.window().scale_factor());
                        let x = (origin.x + rect.size.width / 2.).min(size.width - 24.);
                        let y = origin.y + rect.size.height / 2.;
                        if x >= origin.x && y >= 0. && y < size.height - 40. {
                            Some([x, y])
                        } else {
                            None
                        }
                    };
                    let size = window
                        .window()
                        .size()
                        .to_logical(window.window().scale_factor());
                    eprintln!(
                        "external-ready={}",
                        serde_json::json!({
                            "search": center("Search commands"),
                            "capture": center("Press shortcut keys"),
                            "scale": window.window().scale_factor(),
                            "size": [size.width, size.height],
                        })
                    );
                }
                _ => {}
            }
            // Repeated backend-style refresh while real native input is being delivered.
            // No settings/filesystem service is attached to this standalone probe.
            midnight_ui::refresh(&window, &presenter.borrow());
            let view = window.global::<KeyboardEditorView>();
            let query = view.get_filter();
            let state = serde_json::json!({
                "query": query.as_str(),
                "rows": view.get_commands().row_count(),
                "selected": view.get_selected_id().as_str(),
                "candidate": view.get_candidate().as_str(),
                "capturing": view.get_capturing(),
                "selected_row": view.get_selected_row(),
            })
            .to_string();
            if previous.as_ref() != Some(&state) {
                eprintln!("external-state={state}");
                previous = Some(state);
            }
            if query == "keyboard-native-complete" {
                assert_eq!(view.get_commands().row_count(), 0);
                eprintln!("External native keyboard search completed");
                slint::quit_event_loop().unwrap();
            }
            assert!(
                step < 3000,
                "native input driver did not complete within 60 seconds"
            );
            step += 1;
        },
    );
    window.run()?;
    Ok(())
}
