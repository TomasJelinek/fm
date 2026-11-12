//! Compact Jobs geometry uses its native body while main/Options retain their fixture.
use crate::{inside_window, jobs_form, ui};
use i_slint_core::items::AccessibleRole;
use midnight_ui::OperationRow;
use slint::{ComponentHandle, VecModel};
use std::rc::Rc;

pub fn check(
    size: (usize, usize),
    density: i32,
    font: i32,
    row: impl FnOnce() -> OperationRow + Send + 'static,
    actions: &[&str],
    snapshot: &str,
) {
    let actions: Vec<String> = actions.iter().map(|action| (*action).into()).collect();
    let snapshot = snapshot.to_owned();
    // The legacy main fixture deliberately reuses one adapter. A separate UI
    // thread gives Jobs its own real adapter without changing unrelated controls.
    std::thread::spawn(move || {
        let mut jobs = jobs_form::TestUi::new();
        jobs.window
            .set_rows(Rc::new(VecModel::from(vec![row()])).into());
        jobs.window
            .invoke_apply_preferences(0, density, "".into(), font, font);
        jobs.resize(size.0, size.1);
        assert_eq!(
            jobs.window.window().size(),
            slint::PhysicalSize::new(size.0 as u32, size.1 as u32)
        );
        for action in actions {
            let item = jobs_form::popup_item(&jobs.window, &action, AccessibleRole::Button);
            assert!(
                item.is_visible(),
                "native Jobs action must be visible: {action}"
            );
            inside_window(size.0, size.1, &item);
            assert!(
                ui::visible_text_values(item)
                    .iter()
                    .any(|text| !text.is_empty()),
                "native Jobs action must retain its caption: {action}"
            );
        }
        jobs.snapshot(&snapshot);
        jobs_form::click_popup(&jobs.window, "Close", AccessibleRole::Button);
        assert!(
            !jobs.window.window().is_visible(),
            "actual native Jobs Close must hide the window"
        );
    })
    .join()
    .unwrap();
}
