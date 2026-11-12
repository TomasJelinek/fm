//! Exercise the installed presentation bridge, including retired occurrence tokens.
use super::*;
use i_slint_core::window::WindowInner;
use midnight_core::PaneId;
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
fn installed_filter_and_help_use_only_main_window_and_reject_retired_tokens() {
    std::thread::spawn(|| {
        let adapters = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(Boundary(adapters.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let root = std::env::temp_dir().join("embedded-filter-lifetime");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let out = commands.clone();
        install(
            &main,
            state.clone(),
            Rc::new(move |command| {
                out.borrow_mut().push(command);
                true
            }),
        );
        state.borrow_mut().open_panel_filter(PaneId::Left);
        crate::refresh(&main, &state.borrow());
        let old = main.global::<PanelFilterPresentation>().get_token();
        assert!(!old.is_empty());
        slint::platform::update_timers_and_animations();
        assert_eq!(
            WindowInner::from_pub(main.window()).active_popups().len(),
            1
        );
        assert_eq!(adapters.get(), 1, "filter must use the main adapter");

        state.borrow_mut().cancel_panel_filter();
        state.borrow_mut().open_panel_filter(PaneId::Right);
        crate::refresh(&main, &state.borrow());
        let presentation = main.global::<PanelFilterPresentation>();
        let current = presentation.get_token();
        assert_ne!(old, current);
        presentation.invoke_apply(old.clone(), true, 0, "wrong".into(), false);
        presentation.invoke_clear(old.clone());
        presentation.invoke_dismiss(old.clone());
        presentation.invoke_show_help(old);
        assert!(commands.borrow().is_empty());
        assert!(presentation.get_help_token().is_empty());

        presentation.invoke_show_help(current.clone());
        let old_help = presentation.get_help_token();
        assert!(!old_help.is_empty());
        slint::platform::update_timers_and_animations();
        assert_eq!(
            WindowInner::from_pub(main.window()).active_popups().len(),
            2
        );
        assert_eq!(
            adapters.get(),
            1,
            "nested help must stay in the main adapter"
        );
        presentation.invoke_apply(current.clone(), true, 0, "blocked".into(), false);
        presentation.invoke_clear(current.clone());
        presentation.invoke_dismiss(current.clone());
        assert!(state.borrow().filter_dialog.open);
        assert!(
            commands.borrow().is_empty(),
            "help blocks the filter bridge"
        );

        presentation.invoke_dismiss_help(current.clone(), old_help.clone());
        assert!(presentation.get_help_token().is_empty());
        presentation.invoke_show_help(current.clone());
        let current_help = presentation.get_help_token();
        assert_ne!(old_help, current_help);
        presentation.invoke_dismiss_help(current.clone(), old_help);
        assert_eq!(presentation.get_help_token(), current_help);
        presentation.invoke_dismiss_help(current.clone(), current_help);
        presentation.invoke_apply(current.clone(), true, 0, "*.rs".into(), false);
        assert!(
            matches!(commands.borrow().as_slice(), [Command::ValidatePanelFilter(request)]
            if request.pane == PaneId::Right && request.filter.expression == "*.rs")
        );
        presentation.invoke_dismiss(current.clone());
        assert!(!state.borrow().filter_dialog.open);
        assert!(presentation.get_token().is_empty());
        presentation.invoke_apply(current, true, 0, "retired".into(), false);
        assert_eq!(commands.borrow().len(), 1);
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn shutdown_retires_filter_help_and_prevents_refresh_from_reopening() {
    std::thread::spawn(|| {
        let adapters = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(Boundary(adapters.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let root = std::env::temp_dir().join("embedded-filter-shutdown");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        install(&main, state.clone(), Rc::new(|_| true));
        state.borrow_mut().open_panel_filter(PaneId::Left);
        crate::refresh(&main, &state.borrow());
        let presentation = main.global::<PanelFilterPresentation>();
        let token = presentation.get_token();
        presentation.invoke_show_help(token.clone());
        let help = presentation.get_help_token();
        assert!(!help.is_empty());
        main.invoke_close_panel_filter_window();
        assert!(!state.borrow().filter_dialog.open);
        assert!(presentation.get_token().is_empty());
        assert!(presentation.get_help_token().is_empty());
        state.borrow_mut().open_panel_filter(PaneId::Left);
        crate::refresh(&main, &state.borrow());
        presentation.invoke_show_help(token.clone());
        presentation.invoke_dismiss_help(token, help);
        assert!(presentation.get_token().is_empty());
        assert!(presentation.get_help_token().is_empty());
        assert_eq!(adapters.get(), 1);
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}
