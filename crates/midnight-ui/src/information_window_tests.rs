//! Exercise stale generated callbacks while a former component remains alive.
use super::*;
use std::cell::Cell;

#[test]
fn stale_notice_callbacks_cannot_copy_or_close_a_new_generation() {
    std::thread::spawn(|| {
        slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let owner = DialogOwner::new(&main);
        let blocked = Rc::new(Cell::new(false));
        let gate = blocked.clone();
        owner.set_input_gate(move |value| gate.set(value));
        let windows = Rc::new(RefCell::new(InformationWindows {
            owner,
            sessions: DialogSessions::default(),
            help: None,
            about: None,
            notice: None,
            pending_notice: None,
            closing: false,
        }));
        let path = std::path::PathBuf::from("native-dialog-fixture");
        let presenter = Presenter::new(path.clone(), path);
        let copies = Rc::new(Cell::new(0));
        let observed = copies.clone();
        main.on_copy_message(move || observed.set(observed.get() + 1));
        let authorizations = Rc::new(Cell::new(0));
        let observed = authorizations.clone();
        main.on_authorize_message(move |_| observed.set(observed.get() + 1));
        main.set_message_can_authorize(true);

        main.set_message_title("First notice".into());
        main.set_message_body("First diagnostics".into());
        main.set_message_generation(1);
        open::<NoticeWindow>(&main, &windows, &presenter).unwrap();
        let old = windows
            .borrow()
            .notice
            .as_ref()
            .unwrap()
            .window
            .clone_strong();
        assert!(blocked.get());

        main.set_message_title("Second notice".into());
        main.set_message_body("New diagnostics: C:\\文档\\second.txt".into());
        main.set_message_generation(2);
        open::<NoticeWindow>(&main, &windows, &presenter).unwrap();
        let new = windows
            .borrow()
            .notice
            .as_ref()
            .unwrap()
            .window
            .clone_strong();
        assert!(!old.window().is_visible());
        assert!(new.window().is_visible());

        old.invoke_copy_details();
        old.invoke_authorize_retry();
        old.invoke_request_dismiss();
        assert_eq!(authorizations.get(), 0);
        assert_eq!(copies.get(), 0);
        assert!(new.window().is_visible());
        assert!(blocked.get());
        new.invoke_copy_details();
        assert_eq!(copies.get(), 1);
        assert_eq!(new.get_body(), "New diagnostics: C:\\文档\\second.txt");
        new.invoke_authorize_retry();
        assert_eq!(authorizations.get(), 1);
        new.invoke_authorize_retry();
        assert_eq!(
            authorizations.get(),
            1,
            "a closed dialog cannot authorize twice"
        );
        assert!(!new.window().is_visible());
        assert!(!blocked.get());
        assert!(windows.borrow().notice.is_none());
    })
    .join()
    .unwrap();
}

struct TestPlatform;
impl slint::platform::Platform for TestPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, PlatformError> {
        Ok(
            slint::platform::software_renderer::MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            ),
        )
    }
}
