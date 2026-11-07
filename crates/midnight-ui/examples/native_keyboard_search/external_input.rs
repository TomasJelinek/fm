//! Native message driver handshake. All preferences remain in the probe's presenter.
use super::{AccessibleRole, ComponentHandle, Duration, KeyboardEditorView, MainWindow, ui};
use slint::Model;

pub(super) fn run(window: MainWindow) -> Result<(), Box<dyn std::error::Error>> {
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
                    let center = |label| {
                        let item = ui::popup_item(&window, label, AccessibleRole::TextInput);
                        let rect = item.geometry();
                        let origin = item.map_to_native_window(rect.origin);
                        [
                            origin.x + rect.size.width / 2.,
                            origin.y + rect.size.height / 2.,
                        ]
                    };
                    eprintln!(
                        "external-ready={}",
                        serde_json::json!({
                            "search": center("Search commands"),
                            "capture": center("Press shortcut keys"),
                            "scale": window.window().scale_factor(),
                        })
                    );
                }
                _ => {}
            }
            let view = window.global::<KeyboardEditorView>();
            let query = view.get_filter();
            let state = serde_json::json!({
                "query": query.as_str(),
                "rows": view.get_commands().row_count(),
                "selected": view.get_selected_id().as_str(),
                "candidate": view.get_candidate().as_str(),
                "capturing": view.get_capturing(),
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
