#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{ViewerPluginRow, ViewerPluginsView};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use ui::{TestUi, click_popup};

#[test]
fn plugin_controls_toggle_only_available_packages_and_request_refresh() {
    let mut ui = TestUi::new();
    let toggles = Rc::new(RefCell::new(Vec::new()));
    let captured = toggles.clone();
    let view = ui.window.global::<ViewerPluginsView>();
    view.on_toggle(move |id, enabled| captured.borrow_mut().push((id.to_string(), enabled)));
    let refreshes = Rc::new(Cell::new(0));
    let captured = refreshes.clone();
    view.on_refresh(move || captured.set(captured.get() + 1));
    view.set_settings_ready(true);
    view.set_rows(ModelRc::new(VecModel::from(vec![
        ViewerPluginRow {
            id: "midnight.viewer.source".into(),
            name: "Source viewer".into(),
            version: "1.0".into(),
            status: "Available · not loaded".into(),
            enabled: true,
            configurable: true,
            ..Default::default()
        },
        ViewerPluginRow {
            name: "Broken package".into(),
            status: "Unavailable: incompatible ABI".into(),
            ..Default::default()
        },
    ])));
    ui.window.invoke_show_options();
    ui.window.set_options_topic_index(5);
    ui.draw();
    click_popup(&ui.window, "Source viewer · 1.0", AccessibleRole::Checkbox);
    assert_eq!(
        *toggles.borrow(),
        [("midnight.viewer.source".to_owned(), false)]
    );
    click_popup(&ui.window, "Broken package", AccessibleRole::Checkbox);
    assert_eq!(
        toggles.borrow().len(),
        1,
        "invalid packages must not be configurable"
    );
    click_popup(&ui.window, "Refresh plugins", AccessibleRole::Button);
    assert_eq!(refreshes.get(), 1);
}
