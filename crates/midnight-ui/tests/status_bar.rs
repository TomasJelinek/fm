//! Filename priority and exact clipboard identities at real panel widths.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::DetailRow;
use slint::{ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

#[test]
fn filename_uses_available_width_before_metadata_and_copies_its_full_identity() {
    let mut ui = ui::TestUi::new();
    let name = "長い名前-source-configuration-release-candidate-2026-with-project-and-platform-details.txt";
    ui.window.set_left_details(ModelRc::new(VecModel::from(vec![
        DetailRow {
            identity: "captured-name".into(),
            label: "Name".into(),
            value: name.into(),
            ..Default::default()
        },
        DetailRow {
            identity: "captured-size".into(),
            label: "Bytes".into(),
            value: "123,456,789".into(),
            ..Default::default()
        },
    ])));
    ui.window.set_left_free_space("9.5 GB free".into());
    ui.draw();
    let label = format!("Name: {name}. Copy to clipboard");
    let item = ui::root_item(&ui.window, &label, AccessibleRole::Button);
    let wide = item.geometry().size.width;
    assert!(
        wide > 300.0,
        "filename must not retain the old 220px cap: {wide}"
    );
    render::snapshot(&ui, "status-filename-wide");

    let copied = Rc::new(RefCell::new(Vec::new()));
    let received = copied.clone();
    ui.window.on_detail_copy(move |pane, identity| {
        received.borrow_mut().push((pane, identity.to_string()));
    });
    ui.resize(760, 600);
    let item = ui::root_item(&ui.window, &label, AccessibleRole::Button);
    let geometry = item.geometry();
    let origin = item.map_to_native_window(geometry.origin);
    assert!(geometry.size.width > 100.0);
    assert!(geometry.size.width < wide);
    assert!(origin.x + geometry.size.width <= 382.0);
    render::snapshot(&ui, "status-filename-narrow");
    ui::click_root(&ui.window, &label, AccessibleRole::Button);
    assert_eq!(&*copied.borrow(), &[(0, "captured-name".into())]);
}
