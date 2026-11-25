//! Follow the system appearance reported by Slint's native backend. The widget
//! palette is an output of our theme, so it cannot serve as the system input.
use crate::{MainWindow, Presenter, refresh};
use i_slint_core::{items::ColorScheme, properties::ChangeTracker, window::WindowInner};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

/// Resample after the backend creates the native window, before its first frame.
pub fn sync_host_appearance(window: &MainWindow, presenter: &RefCell<Presenter>) {
    let scheme = WindowInner::from_pub(window.window())
        .context()
        .color_scheme(None);
    apply(window, presenter, scheme);
}

fn apply(window: &MainWindow, presenter: &RefCell<Presenter>, scheme: ColorScheme) {
    let changed = presenter
        .borrow_mut()
        .set_host_dark(scheme == ColorScheme::Dark);
    if changed {
        refresh(window, &presenter.borrow());
    }
}

/// Apply the host appearance immediately and follow native changes for the
/// lifetime of the returned observer. A desktop with no preference uses light.
#[must_use = "retain the observer while the main window is running"]
pub fn observe_host_appearance(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
) -> impl Drop + use<> {
    let context = WindowInner::from_pub(window.window()).context().clone();
    apply(window, presenter, context.color_scheme(None));
    let weak = window.as_weak();
    let presenter = Rc::downgrade(presenter);
    let tracker = ChangeTracker::default();
    tracker.init(
        context,
        |ctx| ctx.color_scheme(None),
        move |_, scheme| {
            if let (Some(window), Some(presenter)) = (weak.upgrade(), presenter.upgrade()) {
                apply(&window, &presenter, *scheme);
            }
        },
    );
    tracker
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_harness::Harness;
    use midnight_core::{Command, PreferenceTheme, UserPreferences};

    #[test]
    fn native_scheme_changes_repaint_the_saved_theme_without_saving() {
        let h = Harness::new(UserPreferences {
            dark_theme: PreferenceTheme::PlumSmoke,
            light_theme: PreferenceTheme::Iris,
            ..Default::default()
        });
        let context = WindowInner::from_pub(h.window.window()).context();
        context.set_color_scheme(ColorScheme::Light);
        let _observer = observe_host_appearance(&h.window, &h.presenter);
        assert_eq!(h.window.get_theme_index(), PreferenceTheme::Iris.index());
        for (mode, expected) in [
            (ColorScheme::Dark, PreferenceTheme::PlumSmoke),
            (ColorScheme::Light, PreferenceTheme::Iris),
        ] {
            context.set_color_scheme(mode);
            ChangeTracker::run_change_handlers();
            assert_eq!(h.window.get_theme_index(), expected.index());
        }
        assert!(
            !h.sent
                .borrow()
                .iter()
                .any(|cmd| matches!(cmd, Command::SavePreferences { .. }))
        );
    }
}
