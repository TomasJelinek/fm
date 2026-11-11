//! Native Filter and Hot Paths preserve their captured pane and location.
#[path = "support/modeless.rs"]
mod modeless;
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{Command, Event, PaneId, PanelFilter, UserPreferences};
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, SharedString,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

struct Boundary {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    refuse_window: Rc<Cell<bool>>,
    attempts: Rc<Cell<usize>>,
}
impl slint::platform::Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        self.attempts.set(self.attempts.get() + 1);
        if self.refuse_window.replace(false) {
            return Err("injected native creation failure".into());
        }
        let native = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::NewBuffer,
        );
        self.windows.borrow_mut().push(native.clone());
        Ok(native)
    }
}

struct Harness {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    commands: Rc<RefCell<Vec<Command>>>,
    paths: [PathBuf; 2],
    refuse_window: Rc<Cell<bool>>,
    attempts: Rc<Cell<usize>>,
}
impl Harness {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        let refuse_window = Rc::new(Cell::new(false));
        let attempts = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(Boundary {
            windows: windows.clone(),
            refuse_window: refuse_window.clone(),
            attempts: attempts.clone(),
        }))
        .unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let paths = [
            std::env::temp_dir().join("filter-original 文档"),
            std::env::temp_dir().join("filter-other"),
        ];
        let state = Rc::new(RefCell::new(Presenter::new(
            paths[0].clone(),
            paths[1].clone(),
        )));
        let mut preferences = UserPreferences::default();
        for (tab, path) in preferences.tabs.iter_mut().zip(&paths) {
            tab.paths = vec![path.clone()];
        }
        preferences.panel_filters[0] = PanelFilter {
            enabled: true,
            expression: "*.txt".into(),
            ..Default::default()
        };
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &main,
            state.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        modeless::draw(&windows.borrow()[0], 1000, 700, "panel-native-owner");
        commands.borrow_mut().clear();
        Self {
            main,
            state,
            windows,
            commands,
            paths,
            refuse_window,
            attempts,
        }
    }
    fn child(&self) -> Rc<MinimalSoftwareWindow> {
        let visible: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|w| w.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(
            visible.len(),
            1,
            "the panel form must have its own native window adapter"
        );
        let child = visible[0].clone();
        child
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        modeless::draw(&child, 560, 440, "panel-native-child");
        assert!(self.main.get_dialog_input_blocked());
        child
    }
}
fn key(window: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
fn value(window: &MinimalSoftwareWindow) -> SharedString {
    modeless::accessible_item(window, AccessibleRole::TextInput, "File filter expression")
        .unwrap()
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap()
}
fn replace(window: &MinimalSoftwareWindow, text: &str) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(window, "a");
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    key(window, text);
}
#[test]
fn native_filter_retains_text_on_resize_and_cancels_pending_validation() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let child = h.child();
    forms::click_control(&child, "File filter expression", AccessibleRole::TextInput);
    replace(&child, "*.文档");
    assert_eq!(
        value(&child),
        "*.文档",
        "the editor accepted the draft before resizing"
    );
    h.main.invoke_activate(1);
    modeless::draw(&child, 320, 200, "native-filter-minimum");
    refresh(&h.main, &h.state.borrow());
    assert_eq!(value(&child), "*.文档");
    modeless::click(&child, "OK");
    let request = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidatePanelFilter(request) => Some(request.clone()),
            _ => None,
        })
        .expect("OK must validate captured filter");
    assert_eq!(request.pane, PaneId::Left);
    assert_eq!(request.filter.expression, "*.文档");
    child.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!child.window().is_visible());
    assert!(!h.main.get_dialog_input_blocked());
    h.state.borrow_mut().apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    h.state
        .borrow_mut()
        .flush_filters(&|_| panic!("a canceled validation must not be saved"));
    h.main.invoke_open_panel_filter(0);
    let next = h.child();
    assert_eq!(value(&next), "*.txt");
    key(&next, Key::Escape);
    assert!(!next.window().is_visible());
}
#[test]
fn native_filter_navigation_retires_form_without_clearing_applied_filter() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let child = h.child();
    h.main.invoke_navigate(
        0,
        h.paths[0]
            .join("new-navigation")
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    assert!(
        !child.window().is_visible(),
        "navigation retires the old native filter"
    );
    assert!(!h.main.get_panel_filter_view().open);
    assert!(
        h.main.get_panel_filter_view().enabled,
        "navigation must not clear the applied filter"
    );
    h.main.invoke_open_panel_filter(0);
    let current = h.child();
    assert_eq!(value(&current), "*.txt");
    key(&current, Key::Escape);
}
#[test]
fn native_hot_paths_add_uses_opening_location_and_close_keeps_committed_edits() {
    let h = Harness::new();
    h.main.invoke_show_bookmarks(0);
    let child = h.child();
    h.main.invoke_navigate(
        0,
        h.paths[0].join("later").to_string_lossy().as_ref().into(),
    );
    h.main.invoke_activate(1);
    modeless::click(&child, "Add current");
    modeless::click(&child, "Add current");
    assert_eq!(
        h.state.borrow().bookmarks().len(),
        1,
        "captured location remains unique"
    );
    assert_eq!(h.state.borrow().bookmarks()[0].path, h.paths[0]);
    assert_eq!(h.state.borrow().bookmarks()[0].slot, Some(1));
    modeless::draw(&child, 320, 200, "native-hotpaths-minimum");
    modeless::click(&child, "Close");
    assert!(!child.window().is_visible());
    assert!(!h.main.get_dialog_input_blocked());
    assert_eq!(h.state.borrow().bookmarks().len(), 1);
}
#[path = "support/native_filter_form.rs"]
mod forms;
#[path = "support/native_hotpaths_form.rs"]
mod hotpaths_form;

#[path = "support/native_panel_failure.rs"]
mod failure;
