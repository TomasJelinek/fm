//! Installed Hot Paths preserve their captured location without creating a window.
use super::*;
use i_slint_core::window::WindowInner;
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};

struct Boundary(Rc<Cell<usize>>);
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        self.0.set(self.0.get() + 1);
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

#[test]
fn installed_hot_paths_keep_opening_location_and_ignore_retired_callbacks() {
    std::thread::spawn(|| {
        let adapters = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(Boundary(adapters.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let root = std::env::temp_dir().join("embedded-hotpaths-lifetime");
        let state = Rc::new(RefCell::new(Presenter::new(
            root.join("first"),
            root.join("second"),
        )));
        let mut preferences = midnight_core::UserPreferences::default();
        preferences.tabs[0].paths = vec![root.join("first")];
        preferences.tabs[1].paths = vec![root.join("second")];
        state
            .borrow_mut()
            .apply(midnight_core::Event::PreferencesLoaded(Ok(Box::new(
                preferences,
            ))));
        install(&main, state.clone(), Rc::new(|_| true));
        crate::refresh(&main, &state.borrow());
        main.invoke_open_hot_paths_window(0);
        let presentation = main.global::<HotPathsPresentation>();
        let old = presentation.get_token();
        assert!(!old.is_empty());
        slint::platform::update_timers_and_animations();
        assert_eq!(
            WindowInner::from_pub(main.window()).active_popups().len(),
            1
        );
        assert_eq!(adapters.get(), 1);
        main.invoke_open_hot_paths_window(1);
        assert_eq!(presentation.get_token(), old, "reopening cannot recapture");
        presentation.invoke_add_bookmark(old.clone());
        assert_eq!(state.borrow().bookmarks()[0].path, root.join("first"));
        presentation.invoke_dismiss(old.clone());
        main.invoke_open_hot_paths_window(1);
        let current = presentation.get_token();
        assert_ne!(old, current);
        presentation.invoke_add_bookmark(old.clone());
        presentation.invoke_remove_bookmark(old.clone(), "1".into());
        presentation.invoke_assign_bookmark(old.clone(), "1".into(), 2);
        presentation.invoke_dismiss(old);
        assert_eq!(state.borrow().bookmarks().len(), 1);
        assert_eq!(presentation.get_token(), current);
        presentation.invoke_add_bookmark(current.clone());
        assert_eq!(state.borrow().bookmarks()[1].path, root.join("second"));
        presentation.invoke_dismiss(current.clone());
        assert_eq!(
            state.borrow().bookmarks().len(),
            2,
            "Close keeps saved changes"
        );
        main.invoke_close_hot_paths_window();
        main.invoke_open_hot_paths_window(0);
        presentation.invoke_add_bookmark(current);
        assert!(presentation.get_token().is_empty());
        assert_eq!(adapters.get(), 1);
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}
