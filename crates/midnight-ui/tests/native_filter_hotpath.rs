//! Installed Filter and Hot Paths use the main adapter and preserve captured authority.
#[path = "support/modeless.rs"]
mod modeless;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ParentItemTraversalMode,
    items::{AccessibleRole, Clip},
    window::WindowInner,
};
use midnight_core::{Command, Event, FilterMode, PaneId, PanelFilter, UserPreferences};
use midnight_ui::{
    HotPathsPresentation, MainWindow, PanelFilterPresentation, Presenter, Theme, install, refresh,
};
use slint::{
    ComponentHandle, SharedString,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
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
    accept_commands: Rc<Cell<bool>>,
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
        preferences.panel_filter_history = vec![PanelFilter {
            enabled: true,
            mode: FilterMode::Regex,
            expression: r"\.TXT$".into(),
            case_sensitive: true,
        }];
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let accept_commands = Rc::new(Cell::new(true));
        let sent = commands.clone();
        let accept = accept_commands.clone();
        install(
            &main,
            state.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                accept.get()
            }),
        );
        // The installed host requests both initial listings. Complete those
        // exact identities before testing actions that require a ready panel.
        let initial_listings: Vec<_> = commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane, navigation, ..
                } => Some((*pane, *navigation)),
                _ => None,
            })
            .collect();
        assert_eq!(initial_listings.len(), 2);
        for (pane, navigation) in initial_listings {
            state.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                entries: Arc::from([]),
                index: Arc::default(),
            });
        }
        refresh(&main, &state.borrow());
        modeless::draw(&windows.borrow()[0], 1000, 700, "embedded-panel-main");
        commands.borrow_mut().clear();
        Self {
            main,
            state,
            windows,
            commands,
            accept_commands,
            paths,
            refuse_window,
            attempts,
        }
    }
    fn form(&self) -> Rc<MinimalSoftwareWindow> {
        let main = self.windows.borrow()[0].clone();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        slint::platform::update_timers_and_animations();
        modeless::draw(&main, 1000, 700, "embedded-panel-popup");
        assert_eq!(
            self.windows.borrow().len(),
            1,
            "form must use the main adapter"
        );
        assert_eq!(self.attempts.get(), 1, "no child Window may be attempted");
        assert!(
            !WindowInner::from_pub(main.window())
                .active_popups()
                .is_empty(),
            "the installed form must be an active popup"
        );
        main
    }
    fn filter_closed(&self) {
        assert!(
            self.main
                .global::<PanelFilterPresentation>()
                .get_token()
                .is_empty()
        );
        assert!(!self.main.get_panel_filter_view().open);
        assert!(
            self.main.window().is_visible(),
            "closing a form keeps Main open"
        );
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
    slint::platform::update_timers_and_animations();
}
fn value(window: &MinimalSoftwareWindow) -> SharedString {
    modeless::accessible_popup_item(window, AccessibleRole::TextInput, "File filter expression")
        .or_else(|| {
            modeless::accessible_item(window, AccessibleRole::TextInput, "File filter expression")
        })
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

fn assert_control_in_clips(window: &MinimalSoftwareWindow, role: AccessibleRole, label: &str) {
    let item = modeless::accessible_popup_item(window, role, label).unwrap();
    assert!(
        item.is_visible(),
        "{label} must be visible at this viewport"
    );
    let bounds = item.geometry();
    let origin = item.map_to_native_window(bounds.origin);
    let mut clipped = false;
    let mut ancestor = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(parent) = ancestor {
        if parent
            .downcast::<Clip>()
            .is_some_and(|clip| clip.as_pin_ref().clip())
        {
            clipped = true;
            let clip_bounds = parent.geometry();
            let clip_origin = parent.map_to_native_window(clip_bounds.origin);
            assert!(
                origin.x >= clip_origin.x - 0.5
                    && origin.y >= clip_origin.y - 0.5
                    && origin.x + bounds.width() <= clip_origin.x + clip_bounds.width() + 0.5
                    && origin.y + bounds.height() <= clip_origin.y + clip_bounds.height() + 0.5,
                "{label} must remain inside its clipped dialog body: control={origin:?} {bounds:?}, clip={clip_origin:?} {clip_bounds:?}"
            );
        }
        ancestor = parent.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    assert!(
        clipped,
        "the installed Filter must retain its clipping frame"
    );
}

#[test]
fn embedded_filter_retains_text_on_resize_and_cancels_pending_validation() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    // Controls must fit the actual clipped body, even though the popup fills Main.
    for (role, label) in [
        (AccessibleRole::TextInput, "File filter expression"),
        (AccessibleRole::Button, "Help"),
        (AccessibleRole::Button, "OK"),
        (AccessibleRole::Button, "Cancel"),
    ] {
        assert_control_in_clips(&main, role, label);
    }
    forms::click_control(&main, "File filter expression", AccessibleRole::TextInput);
    replace(&main, "*.文档");
    assert_eq!(value(&main), "*.文档");
    h.main.invoke_activate(1);
    modeless::draw(&main, 320, 200, "embedded-filter-minimum");
    refresh(&h.main, &h.state.borrow());
    assert_control_in_clips(&main, AccessibleRole::Button, "Cancel");
    assert_eq!(value(&main), "*.文档");
    modeless::click(&main, "OK");
    let request = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidatePanelFilter(request) => Some(request.clone()),
            _ => None,
        })
        .expect("OK must validate the captured filter");
    assert_eq!(request.pane, PaneId::Left);
    assert_eq!(request.filter.expression, "*.文档");
    key(&main, Key::Escape);
    h.filter_closed();
    h.state.borrow_mut().apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    h.state
        .borrow_mut()
        .flush_filters(&|_| panic!("canceled validation must not be saved"));
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    assert_eq!(value(&main), "*.txt");
    key(&main, Key::Escape);
    h.filter_closed();
}

#[test]
fn embedded_filter_navigation_retires_form_without_clearing_applied_filter() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    h.main.invoke_navigate(
        0,
        h.paths[0]
            .join("new-navigation")
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    h.filter_closed();
    assert!(
        h.main.get_panel_filter_view().enabled,
        "navigation preserves the applied filter"
    );
    h.main.invoke_open_panel_filter(0);
    h.form();
    assert_eq!(value(&main), "*.txt");
    key(&main, Key::Escape);
    h.filter_closed();
}

#[test]
fn embedded_hot_paths_add_uses_opening_location_and_close_keeps_saved_edits() {
    let h = Harness::new();
    h.main.invoke_show_bookmarks(0);
    let main = h.form();
    h.main.invoke_navigate(
        0,
        h.paths[0].join("later").to_string_lossy().as_ref().into(),
    );
    h.main.invoke_activate(1);
    modeless::click(&main, "Add current");
    modeless::click(&main, "Add current");
    assert_eq!(
        h.state.borrow().bookmarks().len(),
        1,
        "captured location stays unique"
    );
    assert_eq!(h.state.borrow().bookmarks()[0].path, h.paths[0]);
    assert_eq!(h.state.borrow().bookmarks()[0].slot, Some(1));
    modeless::draw(&main, 320, 200, "embedded-hotpaths-minimum");
    modeless::click(&main, "Close");
    assert!(
        h.main
            .global::<HotPathsPresentation>()
            .get_token()
            .is_empty()
    );
    assert!(main.window().is_visible());
    assert_eq!(h.state.borrow().bookmarks().len(), 1);
}

#[test]
fn embedded_filter_help_blocks_filter_input_and_escape_restores_the_draft_focus() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    forms::click_control(&main, "File filter expression", AccessibleRole::TextInput);
    replace(&main, "*.draft");
    key(&main, Key::F1);
    modeless::draw(&main, 1000, 700, "embedded-filter-expression-help");
    let presentation = h.main.global::<PanelFilterPresentation>();
    assert!(!presentation.get_help_token().is_empty());
    assert_eq!(
        WindowInner::from_pub(main.window()).active_popups().len(),
        2
    );
    assert_eq!(h.attempts.get(), 1, "Help must not create a native Window");
    h.commands.borrow_mut().clear();

    forms::click_control(&main, "File filter expression", AccessibleRole::TextInput);
    replace(&main, "wrong");
    presentation.invoke_apply(presentation.get_token(), true, 0, "wrong".into(), false);
    presentation.invoke_clear(presentation.get_token());
    presentation.invoke_dismiss(presentation.get_token());
    assert_eq!(value(&main), "*.draft");
    assert!(h.commands.borrow().is_empty());
    assert!(h.main.get_panel_filter_view().open);
    assert!(!presentation.get_help_token().is_empty());

    key(&main, Key::Escape);
    assert!(presentation.get_help_token().is_empty());
    assert!(!presentation.get_token().is_empty());
    assert_eq!(value(&main), "*.draft");
    // No pointer repair: Escape must return the original editor's focus.
    replace(&main, "*.after-help");
    key(&main, Key::Return);
    assert!(h.commands.borrow().iter().any(|command| {
        matches!(command, Command::ValidatePanelFilter(request)
            if request.pane == PaneId::Left && request.filter.expression == "*.after-help")
    }));
    key(&main, Key::Escape);
    h.filter_closed();
}

#[test]
fn embedded_filter_queue_rejection_and_theme_changes_keep_the_same_edited_controls() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    forms::click_control(&main, "File filter expression", AccessibleRole::TextInput);
    replace(&main, "kind = file");
    forms::click_control(&main, "Filter mode", AccessibleRole::Combobox);
    key(&main, Key::DownArrow);
    key(&main, Key::DownArrow);
    key(&main, Key::Return);
    h.accept_commands.set(false);
    modeless::click(&main, "OK");
    assert!(!h.main.get_panel_filter_view().busy);
    assert!(
        h.main
            .get_panel_filter_view()
            .error
            .contains("queue is busy")
    );
    let token = h.main.global::<PanelFilterPresentation>().get_token();
    for (width, height, scheme, density, font) in [
        (320, 200, 1, 1, 32),
        (1000, 700, 1, 1, 32),
        (1000, 700, 2, 0, 12),
    ] {
        let theme = h.main.global::<Theme>();
        theme.set_scheme(scheme);
        theme.set_density(density);
        theme.set_desktop_font_size(font);
        theme.set_touch_font_size(font);
        modeless::draw(&main, width, height, "embedded-filter-feedback-theme");
        // Compact scroll content may be offscreen; the fixed dismiss control must fit.
        assert_control_in_clips(&main, AccessibleRole::Button, "Cancel");
        if width == 1000 && density == 1 {
            assert_control_in_clips(&main, AccessibleRole::TextInput, "File filter expression");
            forms::click_control(&main, "File filter expression", AccessibleRole::TextInput);
            replace(&main, "kind = file");
        }
        assert_eq!(value(&main), "kind = file");
        assert_eq!(
            h.main.global::<PanelFilterPresentation>().get_token(),
            token
        );
    }
    h.accept_commands.set(true);
    modeless::click(&main, "OK");
    assert!(
        matches!(h.commands.borrow().last(), Some(Command::ValidatePanelFilter(request))
        if request.filter.mode == FilterMode::Expression && request.filter.expression == "kind = file")
    );
    key(&main, Key::Escape);
    h.filter_closed();
}

#[test]
fn embedded_filter_history_recall_and_clear_edit_only_while_show_all_removes_the_filter() {
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    forms::click_control(&main, "Filter history", AccessibleRole::Combobox);
    key(&main, Key::DownArrow);
    key(&main, Key::Return);
    assert_eq!(value(&main), r"\.TXT$");
    assert!(
        h.commands.borrow().is_empty(),
        "history recall does not apply"
    );
    modeless::click(&main, "OK");
    assert!(
        matches!(h.commands.borrow().last(), Some(Command::ValidatePanelFilter(request))
        if request.filter.mode == FilterMode::Regex && request.filter.case_sensitive)
    );
    key(&main, Key::Escape);
    h.main.invoke_open_panel_filter(0);
    h.form();
    h.commands.borrow_mut().clear();
    modeless::click(&main, "Clear filter expression");
    assert_eq!(value(&main), "");
    assert!(h.main.get_panel_filter_view().enabled);
    assert!(h.commands.borrow().is_empty());
    modeless::click(&main, "Show all");
    h.filter_closed();
    assert!(h.main.get_left_filter_label().is_empty());
    assert!(h.commands.borrow().iter().any(|command| {
        matches!(command, Command::SavePreferences { preferences, .. }
            if !preferences.panel_filters[0].enabled
                && preferences.panel_filters[0].expression == "*.txt")
    }));
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::ValidatePanelFilter(_)))
    );
    h.main.invoke_open_panel_filter(0);
    h.form();
    assert!(!h.main.get_panel_filter_view().enabled);
    assert_eq!(
        value(&main),
        "*.txt",
        "Show all retains the reusable expression"
    );
    key(&main, Key::Escape);
    h.filter_closed();
}

#[path = "support/native_panel_failure.rs"]
mod failure;
#[path = "support/native_filter_form.rs"]
mod forms;
#[path = "support/native_hotpaths_form.rs"]
mod hotpaths_form;

#[test]
fn embedded_forms_restore_the_invoking_editor_after_the_popup_close_tick() {
    let h = Harness::new();
    let main = h.windows.borrow()[0].clone();
    h.main.invoke_focus_command();
    key(&main, "before");
    assert_eq!(h.main.get_command_text(), "before");
    h.main.invoke_open_panel_filter(0);
    h.form();
    key(&main, Key::Escape);
    h.filter_closed();
    slint::platform::update_timers_and_animations();
    key(&main, "-filter");
    assert_eq!(h.main.get_command_text(), "before-filter");
    h.main.invoke_show_bookmarks(0);
    h.form();
    key(&main, Key::Escape);
    slint::platform::update_timers_and_animations();
    key(&main, "-paths");
    assert_eq!(h.main.get_command_text(), "before-filter-paths");
}

#[test]
fn retained_filter_and_help_items_cannot_act_on_replacement_popups() {
    use i_slint_core::accessibility::AccessibilityAction;
    let h = Harness::new();
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    let old_buttons: Vec<_> = ["OK", "Cancel", "Cancel dialog", "Help", "Show all"]
        .into_iter()
        .map(|label| modeless::accessible_popup_item(&main, AccessibleRole::Button, label).unwrap())
        .collect();
    key(&main, Key::Escape);
    h.main.invoke_open_panel_filter(1);
    h.form();
    let presentation = h.main.global::<PanelFilterPresentation>();
    let token = presentation.get_token();
    let filter_focus = WindowInner::from_pub(main.window())
        .focus_item
        .borrow()
        .upgrade()
        .unwrap();
    for button in old_buttons {
        button.accessible_action(&AccessibilityAction::Default);
    }
    assert!(
        WindowInner::from_pub(main.window())
            .focus_item
            .borrow()
            .upgrade()
            .is_some_and(|focused| focused == filter_focus),
        "retired Filter controls must not take focus from the current occurrence"
    );
    assert_eq!(presentation.get_token(), token);
    assert!(presentation.get_help_token().is_empty());
    assert!(h.commands.borrow().is_empty());
    modeless::click(&main, "Help");
    modeless::draw(&main, 1000, 700, "embedded-help-first-occurrence");
    let old_close: Vec<_> = ["Close", "Close dialog"]
        .into_iter()
        .map(|label| modeless::accessible_popup_item(&main, AccessibleRole::Button, label).unwrap())
        .collect();
    key(&main, Key::Escape);
    modeless::click(&main, "Help");
    modeless::draw(&main, 1000, 700, "embedded-help-second-occurrence");
    let help = presentation.get_help_token();
    let help_focus = WindowInner::from_pub(main.window())
        .focus_item
        .borrow()
        .upgrade()
        .unwrap();
    for button in old_close {
        button.accessible_action(&AccessibilityAction::Default);
    }
    assert!(
        WindowInner::from_pub(main.window())
            .focus_item
            .borrow()
            .upgrade()
            .is_some_and(|focused| focused == help_focus),
        "retired Help controls must not take focus from the current occurrence"
    );
    assert_eq!(presentation.get_help_token(), help);
    assert!(!help.is_empty());
    key(&main, Key::Escape);
    key(&main, Key::Escape);
    h.filter_closed();
}

#[test]
fn retained_hotpaths_items_cannot_edit_or_close_a_replacement_popup() {
    use i_slint_core::accessibility::AccessibilityAction;
    let h = Harness::new();
    h.main.invoke_show_bookmarks(0);
    let main = h.form();
    let old_add =
        modeless::accessible_popup_item(&main, AccessibleRole::Button, "Add current").unwrap();
    let old_close =
        modeless::accessible_popup_item(&main, AccessibleRole::Button, "Close").unwrap();
    key(&main, Key::Escape);
    h.main.invoke_show_bookmarks(1);
    h.form();
    let presentation = h.main.global::<HotPathsPresentation>();
    let token = presentation.get_token();
    old_add.accessible_action(&AccessibilityAction::Default);
    old_close.accessible_action(&AccessibilityAction::Default);
    assert!(h.state.borrow().bookmarks().is_empty());
    assert_eq!(presentation.get_token(), token);
    modeless::click(&main, "Add current");
    assert_eq!(h.state.borrow().bookmarks()[0].path, h.paths[1]);
    key(&main, Key::Escape);
    assert!(presentation.get_token().is_empty());
}
