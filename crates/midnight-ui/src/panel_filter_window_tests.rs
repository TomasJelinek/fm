//! Exercise the installed presentation bridge, including retired occurrence tokens.
use super::*;
use i_slint_core::window::WindowInner;
use midnight_core::PaneId;
use slint::platform::{
    Platform, WindowAdapter, WindowEvent,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};

struct Boundary {
    adapters: Rc<Cell<usize>>,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        self.adapters.set(self.adapters.get() + 1);
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
}

fn window_fixture() -> (MainWindow, Rc<MinimalSoftwareWindow>, Rc<Cell<usize>>) {
    let adapters = Rc::new(Cell::new(0));
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Boundary {
        adapters: adapters.clone(),
        windows: windows.clone(),
    }))
    .unwrap();
    let main = MainWindow::new().unwrap();
    main.show().unwrap();
    let native = windows.borrow()[0].clone();
    native.set_size(slint::PhysicalSize::new(1000, 700));
    native.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1000., 700.),
    });
    present(&native);
    (main, native, adapters)
}

fn present(native: &MinimalSoftwareWindow) {
    slint::platform::update_timers_and_animations();
    let size = native.window().size();
    assert!(
        size.width > 0 && size.height > 0,
        "a popup needs a live viewport"
    );
    let mut pixels = vec![slint::Rgb8Pixel::default(); size.width as usize * size.height as usize];
    native.window().request_redraw();
    assert!(native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, size.width as usize);
    }));
    slint::platform::update_timers_and_animations();
}

#[test]
fn installed_filter_and_help_use_only_main_window_and_reject_retired_tokens() {
    std::thread::spawn(|| {
        let (main, native, adapters) = window_fixture();
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
        present(&native);
        assert_eq!(
            WindowInner::from_pub(main.window()).active_popups().len(),
            1
        );
        assert_eq!(adapters.get(), 1, "filter must use the main adapter");

        state.borrow_mut().cancel_panel_filter();
        state.borrow_mut().open_panel_filter(PaneId::Right);
        crate::refresh(&main, &state.borrow());
        // Help is a user action on the replacement form, after that form exists.
        present(&native);
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
        present(&native);
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
        let (main, native, adapters) = window_fixture();
        let root = std::env::temp_dir().join("embedded-filter-shutdown");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        install(&main, state.clone(), Rc::new(|_| true));
        state.borrow_mut().open_panel_filter(PaneId::Left);
        crate::refresh(&main, &state.borrow());
        present(&native);
        let presentation = main.global::<PanelFilterPresentation>();
        let token = presentation.get_token();
        presentation.invoke_show_help(token.clone());
        let help = presentation.get_help_token();
        assert!(!help.is_empty());
        present(&native);
        assert_eq!(
            WindowInner::from_pub(main.window()).active_popups().len(),
            2
        );
        main.invoke_close_panel_filter_window();
        present(&native);
        assert!(!state.borrow().filter_dialog.open);
        assert!(presentation.get_token().is_empty());
        assert!(presentation.get_help_token().is_empty());
        assert!(
            WindowInner::from_pub(main.window())
                .active_popups()
                .is_empty()
        );
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
