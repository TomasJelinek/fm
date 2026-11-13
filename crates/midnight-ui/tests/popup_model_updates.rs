//! Model changes must retire row-owned tooltips before Slint instantiates popup children.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::Event;
use midnight_ui::{ColorOptionsView, KeyboardEditorView, Presenter};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn options(topic: i32, clock: Rc<Cell<Duration>>) -> ui::TestUi {
    let mut ui = ui::TestUi::with_clock(Some(clock));
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    midnight_ui::install(&ui.window, presenter.clone(), Rc::new(|_| true));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::refresh(&ui.window, &presenter.borrow());
    ui.window.invoke_show_options();
    ui.window.set_options_topic_index(topic);
    settle(&mut ui);
    ui
}

fn settle(ui: &mut ui::TestUi) {
    for _ in 0..3 {
        ui.draw();
    }
}

fn tooltip_count(window: &midnight_ui::MainWindow) -> usize {
    WindowInner::from_pub(window.window())
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .count()
}

fn hover_selected_row(ui: &mut ui::TestUi, label: &str, clock: &Cell<Duration>) {
    let position = {
        let row = ui::popup_item(&ui.window, label, AccessibleRole::ListItem);
        let rect = row.geometry();
        let origin = row.map_to_native_window(rect.origin);
        slint::LogicalPosition::new(
            origin.x + rect.width() / 2.0,
            origin.y + rect.height() / 2.0,
        )
    }; // Do not retain the row: its delegate must actually be destroyed by the model update.
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    settle(ui);
    clock.set(Duration::from_secs(2));
    settle(ui);
    assert!(
        tooltip_count(&ui.window) > 0,
        "the actual row tooltip must be open"
    );
}

#[test]
fn keyboard_selection_retires_hovered_command_before_replacing_popup_rows() {
    let clock = Rc::new(Cell::new(Duration::ZERO));
    let mut ui = options(3, clock.clone());
    let window = ui.window.clone_strong();
    let view = window.global::<KeyboardEditorView>();
    view.invoke_move_selection(1_000_000);
    settle(&mut ui);
    let previous = view.get_selected_label();
    ui::click_popup(&window, &previous, AccessibleRole::ListItem);
    settle(&mut ui);
    hover_selected_row(&mut ui, &previous, &clock);
    ui::key(&window, Key::UpArrow);
    settle(&mut ui);
    assert_ne!(view.get_selected_label(), previous);
    assert_eq!(tooltip_count(&window), 0);
    assert!(window.get_options_open());
}

#[test]
fn removing_color_rule_retires_its_tooltip_before_replacing_popup_rows() {
    let clock = Rc::new(Cell::new(Duration::ZERO));
    let mut ui = options(8, clock.clone());
    let window = ui.window.clone_strong();
    let view = window.global::<ColorOptionsView>();
    let before = view.get_labels().row_count();
    assert!(before > 1);
    let label = view.get_labels().row_data(0).unwrap();
    ui::click_popup(&window, &label, AccessibleRole::ListItem);
    settle(&mut ui);
    hover_selected_row(&mut ui, &label, &clock);
    view.invoke_action(1);
    settle(&mut ui);
    assert_eq!(view.get_labels().row_count(), before - 1);
    assert_eq!(tooltip_count(&window), 0);
    assert!(window.get_options_open());
}
