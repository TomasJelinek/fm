use super::*;
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
struct Boundary;
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}
#[test]
fn stale_hotpaths_callbacks_never_edit_or_close_a_replacement() {
    std::thread::spawn(|| {
        slint::platform::set_platform(Box::new(Boundary)).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let root = std::env::temp_dir().join("hotpath-lifetime");
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
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(|_| true);
        let closing = Rc::new(Cell::new(false));
        let failures = Failures::new(
            &main,
            &state,
            &send,
            &closing,
            "Hot Paths window",
            |_, _: u8, _| true,
        );
        let ctx = Rc::new(Context {
            main: main.as_weak(),
            state: Rc::downgrade(&state),
            send,
            owner: DialogOwner::new(&main),
            sessions: RefCell::default(),
            record: RefCell::default(),
            closing,
            failures,
        });
        let first = state
            .borrow()
            .capture_bookmark_location(PaneId::Left, "First".into())
            .unwrap();
        *ctx.record.borrow_mut() =
            Some(Rc::new(open(&ctx, &main, &state.borrow(), first).unwrap()));
        let old = ctx.record.borrow().as_ref().unwrap().window.clone_strong();
        old.invoke_request_dismiss();
        let second = state
            .borrow()
            .capture_bookmark_location(PaneId::Right, "Second".into())
            .unwrap();
        *ctx.record.borrow_mut() =
            Some(Rc::new(open(&ctx, &main, &state.borrow(), second).unwrap()));
        let current = ctx.record.borrow().as_ref().unwrap().window.clone_strong();
        old.invoke_add_bookmark();
        old.invoke_remove_bookmark("1".into());
        old.invoke_assign_bookmark("1".into(), 2);
        old.invoke_request_dismiss();
        assert!(state.borrow().bookmarks().is_empty());
        assert!(current.window().is_visible());
        current.invoke_add_bookmark();
        assert_eq!(state.borrow().bookmarks()[0].path, root.join("second"));
        current.invoke_request_dismiss();
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}
