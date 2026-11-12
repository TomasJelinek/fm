//! Embedded notice callbacks are bound to their exact diagnostic generation.
use super::*;
use crate::NoticePresentation;
use i_slint_core::window::WindowInner;
use std::cell::Cell;

struct TestPlatform(Rc<Cell<usize>>);
impl slint::platform::Platform for TestPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, PlatformError> {
        self.0.set(self.0.get() + 1);
        Ok(
            slint::platform::software_renderer::MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            ),
        )
    }
}

#[test]
fn embedded_notice_rejects_stale_copy_authorize_and_close_without_creating_another_window() {
    std::thread::spawn(|| {
        let adapters = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(TestPlatform(adapters.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let context = notices::install(&main);
        let copies = Rc::new(Cell::new(0));
        let observed = copies.clone();
        main.on_copy_message(move || observed.set(observed.get() + 1));
        let authorizations = Rc::new(Cell::new(0));
        let observed = authorizations.clone();
        main.on_authorize_message(move |generation| {
            assert_eq!(generation, 2);
            observed.set(observed.get() + 1);
        });
        main.set_message_can_authorize(true);
        main.set_message_title("First notice".into());
        main.set_message_body("First diagnostics".into());
        main.set_message_generation(1);
        main.invoke_open_notice_window();
        slint::platform::update_timers_and_animations();
        let presentation = main.global::<NoticePresentation>();
        let old = presentation.get_token();
        assert_eq!(old, "1");
        assert_eq!(
            adapters.get(),
            1,
            "a blocking notice stays in its main window"
        );
        assert_eq!(
            WindowInner::from_pub(main.window()).active_popups().len(),
            1
        );
        assert!(main.get_embedded_modal_open());
        assert!(!crate::dialog_host::dialog_input_blocked(main.window()));

        main.set_message_title("Second notice".into());
        main.set_message_body("New diagnostics: C:\\文档\\second.txt".into());
        main.set_message_generation(2);
        main.invoke_open_notice_window();
        slint::platform::update_timers_and_animations();
        let current = presentation.get_token();
        presentation.invoke_copy_details(old.clone());
        presentation.invoke_authorize_retry(old.clone());
        presentation.invoke_dismiss(old);
        assert_eq!(copies.get(), 0);
        assert_eq!(authorizations.get(), 0);
        assert_eq!(presentation.get_token(), current);
        assert_eq!(
            presentation.get_body(),
            "New diagnostics: C:\\文档\\second.txt"
        );
        assert_eq!(adapters.get(), 1);
        presentation.invoke_copy_details(current.clone());
        assert_eq!(copies.get(), 1);
        presentation.invoke_authorize_retry(current.clone());
        presentation.invoke_authorize_retry(current);
        assert_eq!(
            authorizations.get(),
            1,
            "retired notices cannot authorize twice"
        );
        slint::platform::update_timers_and_animations();
        assert!(presentation.get_token().is_empty());
        assert!(
            WindowInner::from_pub(main.window())
                .active_popups()
                .is_empty()
        );
        assert!(!main.get_embedded_modal_open());
        notices::close(&context, &main);
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn startup_notice_presents_only_the_latest_generation_and_shutdown_retires_it() {
    std::thread::spawn(|| {
        let adapters = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(TestPlatform(adapters.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        let context = notices::install(&main);
        main.set_message_generation(1);
        main.invoke_open_notice_window();
        main.set_message_body("Latest startup diagnostic".into());
        main.set_message_generation(2);
        main.invoke_open_notice_window();
        assert!(main.global::<NoticePresentation>().get_token().is_empty());
        main.show().unwrap();
        notices::refresh(&context, &main);
        slint::platform::update_timers_and_animations();
        let presentation = main.global::<NoticePresentation>();
        assert_eq!(presentation.get_token(), "2");
        assert_eq!(presentation.get_body(), "Latest startup diagnostic");
        assert_eq!(adapters.get(), 1);
        notices::close(&context, &main);
        main.invoke_open_notice_window();
        notices::refresh(&context, &main);
        assert!(presentation.get_token().is_empty());
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}
