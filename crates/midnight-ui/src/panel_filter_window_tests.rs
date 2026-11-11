//! Retaining an old native component must not give it a newer filter occurrence.
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
fn stale_filter_callbacks_do_not_cancel_clear_or_submit_a_replacement() {
    std::thread::spawn(|| {
        slint::platform::set_platform(Box::new(Boundary)).unwrap();
        let main=MainWindow::new().unwrap(); main.show().unwrap();
        let root=std::env::temp_dir().join("filter-lifetime");
        let state=Rc::new(RefCell::new(Presenter::new(root.clone(),root)));
        let commands=Rc::new(RefCell::new(Vec::new())); let out=commands.clone();
        let ctx=Rc::new(Context { main:main.as_weak(),state:Rc::downgrade(&state),send:Rc::new(move |c| {out.borrow_mut().push(c);true}),owner:DialogOwner::new(&main),sessions:RefCell::default(),record:RefCell::default(),failed:Cell::new(None),closing:Cell::new(false) });
        let context=ctx.clone();
        main.on_refresh_panel_filter_window(move || { let main=context.main.upgrade().unwrap(); let state=context.state.upgrade().unwrap(); synchronize(&context,&main,&state.borrow()); });
        state.borrow_mut().open_panel_filter(midnight_core::PaneId::Left);
        crate::refresh(&main,&state.borrow());
        let helps=Rc::new(Cell::new(0)); let help_count=helps.clone();
        main.on_open_help_window(move || help_count.set(help_count.get()+1));
        let old=ctx.record.borrow().as_ref().unwrap().window.clone_strong();
        state.borrow_mut().cancel_panel_filter();
        state.borrow_mut().open_panel_filter(midnight_core::PaneId::Right);
        crate::refresh(&main,&state.borrow());
        let new=ctx.record.borrow().as_ref().unwrap().window.clone_strong();
        old.invoke_apply(true,0,"wrong".into(),false);
        old.invoke_clear(); old.invoke_request_dismiss(); old.invoke_help();
        assert_eq!(helps.get(), 0);
        new.invoke_help();
        assert_eq!(helps.get(), 0, "filter help must not use the main window's modeless sibling" );
        assert_eq!(ctx.record.borrow().as_ref().unwrap().help.borrow().as_ref().unwrap().window.get_topic(), 1);

        let help = ctx.record.borrow().as_ref().unwrap().help.borrow().as_ref().unwrap().window.clone_strong();
        help.invoke_request_dismiss();
        assert!(commands.borrow().is_empty());
        assert!(new.window().is_visible());
        assert!(state.borrow().filter_dialog.open);
        new.invoke_apply(true,0,"*.rs".into(),false);
        assert!(matches!(commands.borrow().as_slice(),[Command::ValidatePanelFilter(request)] if request.pane==midnight_core::PaneId::Right && request.filter.expression=="*.rs"));
        new.invoke_request_dismiss();
        main.hide().unwrap();
    }).join().unwrap();
}

#[test]
fn filter_help_owns_a_nested_modal_lease_and_retires_with_its_exact_filter() {
    std::thread::spawn(|| {
        slint::platform::set_platform(Box::new(Boundary)).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let root = std::env::temp_dir().join("filter-help-lifetime");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        let ctx = Rc::new(Context {
            main: main.as_weak(),
            state: Rc::downgrade(&state),
            send: Rc::new(|_| true),
            owner: DialogOwner::new(&main),
            sessions: RefCell::default(),
            record: RefCell::default(),
            failed: Cell::new(None),
            closing: Cell::new(false),
        });
        let context = ctx.clone();
        main.on_refresh_panel_filter_window(move || {
            let main = context.main.upgrade().unwrap();
            let state = context.state.upgrade().unwrap();
            synchronize(&context, &main, &state.borrow());
        });
        state
            .borrow_mut()
            .open_panel_filter(midnight_core::PaneId::Left);
        crate::refresh(&main, &state.borrow());
        let filter = ctx.record.borrow().as_ref().unwrap().window.clone_strong();
        assert!(crate::dialog_host::dialog_input_blocked(main.window()));
        assert!(!crate::dialog_host::dialog_input_blocked(filter.window()));
        filter.invoke_help();
        let old = ctx
            .record
            .borrow()
            .as_ref()
            .unwrap()
            .help
            .borrow()
            .as_ref()
            .unwrap()
            .window
            .clone_strong();
        assert!(old.window().is_visible());
        assert_eq!(old.get_topic(), 1);
        assert!(crate::dialog_host::dialog_input_blocked(filter.window()));
        assert!(!crate::dialog_host::dialog_input_blocked(old.window()));
        old.invoke_request_dismiss();
        assert!(!old.window().is_visible());
        assert!(!crate::dialog_host::dialog_input_blocked(filter.window()));
        assert!(crate::dialog_host::dialog_input_blocked(main.window()));
        assert!(state.borrow().filter_dialog.open);
        filter.invoke_help();
        let current = ctx
            .record
            .borrow()
            .as_ref()
            .unwrap()
            .help
            .borrow()
            .as_ref()
            .unwrap()
            .window
            .clone_strong();
        old.invoke_request_dismiss();
        assert!(
            current.window().is_visible(),
            "retired Help cannot close a newer occurrence"
        );
        assert!(crate::dialog_host::dialog_input_blocked(filter.window()));
        filter.invoke_request_dismiss();
        assert!(
            !current.window().is_visible(),
            "retiring filter closes its help first"
        );
        assert!(!filter.window().is_visible());
        assert!(!crate::dialog_host::dialog_input_blocked(filter.window()));
        assert!(!crate::dialog_host::dialog_input_blocked(main.window()));
        current.invoke_request_dismiss();
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}
