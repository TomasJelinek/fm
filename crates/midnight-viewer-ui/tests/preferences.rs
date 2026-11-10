//! Preference acceptance must not depend on a document frame or an open plugin.
use midnight_core::*;
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
use slint::{
    ComponentHandle, Model,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

struct TestPlatform;
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

fn open(manager: &ViewerWindows, preferences: ViewerPreferences) -> ViewerWindow {
    let path = std::env::temp_dir().join("viewer-settings-input.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let shortcuts = ShortcutMap::new(&[]).unwrap();
    manager
        .open(
            ViewerLaunch {
                associations: Default::default(),
                tree: None,
                generated: None,
                preferences: Arc::new(preferences),
                focused: path,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                archive: None,
                basic: true,
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [0, 0, 0, 255],
                selection: [80, 80, 80, 255],
                accent: [100, 100, 255, 255],
            },
            slint::LogicalSize::new(900.0, 600.0),
            Rc::new(move |chord, search| {
                shortcuts
                    .resolve(
                        chord,
                        if search {
                            ShortcutContext::ViewerSearch
                        } else {
                            ShortcutContext::Viewer
                        },
                    )
                    .map(|action| action.id.clone())
            }),
            Rc::new(|_| {}),
        )
        .unwrap()
        .upgrade()
        .unwrap()
}

fn escape(window: &ViewerWindow) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!window.window().is_visible());
}

#[test]
fn accepted_settings_survive_immediate_escape_and_reopen_from_two_windows() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // This boundary never reads a real user file, even if plugin discovery succeeds.
    let service = midnight_viewer_engine::Service::with_native_capture(
        runtime.handle().clone(),
        Arc::new(|_, _| Err("Content is deliberately unavailable in this input test.".into())),
    );
    let manager = ViewerWindows::new(service.clone());
    let saved = Rc::new(RefCell::new(ViewerPreferences::default()));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let capture = saved.clone();
    let observed = changes.clone();
    manager.set_preferences_listener(Rc::new(move |change| {
        capture.borrow_mut().apply(&change).unwrap();
        observed.borrow_mut().push(change);
    }));
    let first = open(&manager, saved.borrow().clone());
    let second = open(&manager, saved.borrow().clone());
    first.invoke_change_wrap(true);
    assert!(
        saved.borrow().wrap,
        "accepted before any timer/frame delivery"
    );
    escape(&first);
    second.invoke_change_case_sensitive(true);
    assert!(
        saved.borrow().wrap,
        "another window must not reset earlier edits"
    );
    assert!(saved.borrow().case_sensitive);
    escape(&second);
    assert_eq!(
        *changes.borrow(),
        [
            ViewerPreferenceChange::Wrap(true),
            ViewerPreferenceChange::CaseSensitive(true),
        ]
    );
    manager.close_all();

    // Recreate the window manager so restoration cannot come from its old sessions.
    let reopened_manager = ViewerWindows::new(service);
    let observed = changes.clone();
    reopened_manager.set_preferences_listener(Rc::new(move |change| {
        observed.borrow_mut().push(change);
    }));
    let reopened = open(&reopened_manager, saved.borrow().clone());
    assert!(reopened.get_wrap_enabled());
    assert!(reopened.get_case_sensitive());
    reopened.invoke_change_wrap(true);
    reopened.invoke_change_case_sensitive(true);
    assert_eq!(
        changes.borrow().len(),
        2,
        "unchanged choices do not save again"
    );
    let mut restored = saved.borrow().clone();
    restored.case_sensitive = false;
    reopened_manager.restore_preferences(Arc::new(restored));
    assert!(!reopened.get_case_sensitive());
    assert_eq!(
        changes.borrow().len(),
        2,
        "programmatic restoration is not an edit"
    );
    escape(&reopened);
    reopened_manager.close_all();
}

#[test]
fn source_and_markdown_controls_save_immediately_and_restore_per_mode() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = midnight_viewer_engine::Service::with_native_capture(
        runtime.handle().clone(),
        Arc::new(|_, _| Err("No content required".into())),
    );
    let manager = ViewerWindows::new(service.clone());
    let saved = Rc::new(RefCell::new(ViewerPreferences::default()));
    let target = saved.clone();
    manager.set_preferences_listener(Rc::new(move |change| {
        target.borrow_mut().apply(&change).unwrap()
    }));
    let source = open(&manager, saved.borrow().clone());
    source.set_source_mode(true);
    source.invoke_change_line_numbers(false);
    source.invoke_change_font_family("Consolas".into());
    source.invoke_change_font_size(19);
    escape(&source);
    let markdown = open(&manager, saved.borrow().clone());
    markdown.set_markdown_mode(true);
    markdown.invoke_change_markdown_preset(2);
    markdown.invoke_change_font_family("Noto Sans CJK JP".into());
    markdown.invoke_change_font_size(23);
    markdown.invoke_change_font_size(80);
    escape(&markdown);
    assert!(!saved.borrow().line_numbers);
    assert_eq!(saved.borrow().source_font.family, "Consolas");
    assert_eq!(saved.borrow().source_font.size, 19);
    assert_eq!(saved.borrow().markdown_font.family, "Noto Sans CJK JP");
    assert_eq!(saved.borrow().markdown_font.size, 23);
    assert_eq!(saved.borrow().markdown_preset, MarkdownPreset::Comfortable);
    assert_eq!(
        saved.borrow().text_font,
        ViewerPreferences::default().text_font
    );
    manager.close_all();
    let reopened = ViewerWindows::new(service);
    let window = open(&reopened, saved.borrow().clone());
    window.set_markdown_mode(true);
    reopened.restore_preferences(Arc::new(saved.borrow().clone()));
    assert!(!window.get_line_numbers());
    assert_eq!(window.get_viewer_font_family(), "Noto Sans CJK JP");
    assert_eq!(
        window
            .get_font_families()
            .row_data(window.get_font_family_index() as usize)
            .unwrap(),
        "Noto Sans CJK JP",
        "restoration must select the saved family even when it is unavailable locally"
    );
    assert_eq!(window.get_viewer_font_size(), 23);
    assert_eq!(window.get_markdown_preset(), 2);
    window.set_markdown_mode(false);
    window.set_source_mode(true);
    reopened.restore_preferences(Arc::new(saved.borrow().clone()));
    assert_eq!(window.get_viewer_font_family(), "Consolas");
    assert_eq!(window.get_viewer_font_size(), 19);
    reopened.close_all();
}
