//! Prefix search follows actual file-panel text focus and immutable backend snapshots.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::input::{InternalKeyEvent, KeyEvent, KeyEventType};
use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, Theme, install, refresh};
use slint::{
    ComponentHandle, Model, SharedString,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use ui::platform_keys;
#[cfg(windows)]
#[path = "support/windows_search_keys.rs"]
mod search_keys;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn key(window: &MainWindow, text: impl Into<SharedString>) {
    platform_keys::key(window, text.into());
}

fn chord(window: &MainWindow, modifier: Key, text: impl Into<SharedString>) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: modifier.into(),
    });
    key(window, text);
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: modifier.into(),
    });
}

fn latest_search(commands: &RefCell<Vec<Command>>) -> SearchRequest {
    commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Search(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap()
}

fn search_count(commands: &RefCell<Vec<Command>>) -> usize {
    commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::Search(_)))
        .count()
}

fn accept(
    window: &MainWindow,
    presenter: &RefCell<Presenter>,
    commands: &RefCell<Vec<Command>>,
    request: &SearchRequest,
    rows: &[usize],
    end: usize,
) {
    let matched: Vec<_> = rows
        .iter()
        .map(|row| request.source[*row].clone())
        .collect();
    presenter.borrow_mut().apply_with_sender(
        Event::SearchReady {
            pane: request.pane,
            navigation: request.navigation,
            revision: request.revision,
            result: Arc::new(SearchResult {
                source: request.source.clone(),
                filtered: request
                    .source
                    .iter()
                    .enumerate()
                    .map(|(source_row, _)| SearchMatch {
                        source_row,
                        display_prefix_end: 0,
                    })
                    .collect::<Vec<_>>()
                    .into(),
                filtered_index: Arc::new(DirectoryIndex::from_entries(&request.source)),
                matches: rows
                    .iter()
                    .map(|row| SearchMatch {
                        source_row: *row,
                        display_prefix_end: end,
                    })
                    .collect::<Vec<_>>()
                    .into(),
                index: Arc::new(DirectoryIndex::from_entries(&matched)),
            }),
        },
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    refresh(window, &presenter.borrow());
}

fn composition(window: &MainWindow, kind: KeyEventType, text: &str, preedit: &str) {
    let mut key_event = KeyEvent::default();
    key_event.text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::internal(InternalKeyEvent {
            key_event,
            event_type: kind,
            preedit_text: preedit.into(),
            ..Default::default()
        }));
}

fn row_pointer(window: &MainWindow, row: usize, down: bool) {
    let theme = window.global::<Theme>();
    let top = window.get_left_list_top();
    let position =
        slint::LogicalPosition::new(100.0, top + (row as f32 + 0.5) * theme.get_row_height());
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(if down {
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        }
    } else {
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        }
    });
}

#[test]
fn plain_typing_searches_the_focused_panes_original_snapshot_without_navigating() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::temp_dir().join("midnight-panel-search 文档");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    let sources: [Arc<[Entry]>; 2] = std::array::from_fn(|pane| {
        ["alpha.txt", "alphabet.txt", "beta.txt"]
            .into_iter()
            .enumerate()
            .map(|(row, name)| {
                Entry::new(
                    EntryId((pane * 100 + row + 1) as u64),
                    root.join(name),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into()
    });
    for pane in [PaneId::Left, PaneId::Right] {
        let navigation = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: target,
                    navigation,
                    ..
                } if *target == pane => Some(*navigation),
                _ => None,
            })
            .unwrap();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            entries: sources[pane.index()].clone(),
            index: Arc::new(DirectoryIndex::from_entries(&sources[pane.index()])),
        });
    }
    refresh(&window, &presenter.borrow());
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    window.invoke_focus_files();
    let before = commands.borrow().len();
    for character in ["a", "l"] {
        platform_keys::key(&window, character.into());
    }
    let recorded = commands.borrow();
    let request = recorded[before..]
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Search(request) => Some(request.clone()),
            _ => None,
        })
        .expect("plain typing in files must request an asynchronous prefix search");
    assert_eq!(request.pane, PaneId::Left);
    assert_eq!(request.query, "al");
    assert_eq!(request.location, PathBuf::from(&root));
    assert!(Arc::ptr_eq(&request.source, &sources[0]));
    assert_eq!(window.get_focused_pane(), 0);
    assert!(recorded[before..].iter().all(|command| !matches!(
        command,
        Command::Navigate { .. } | Command::Refresh { .. } | Command::LaunchTerminal { .. }
    )));
    drop(recorded);
    let count = commands.borrow().len();
    key(&window, Key::Return);
    chord(&window, Key::Alt, Key::Return);
    assert_eq!(
        commands.borrow().len(),
        count,
        "pending search cannot activate a leftover row"
    );

    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    row_pointer(&window, 2, true);
    accept(&window, &presenter, &commands, &request, &[0, 1], 2);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    row_pointer(&window, 2, false);
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    assert!(
        commands.borrow().iter().any(|command| matches!(command,
        Command::Selection(request) if matches!(&request.action,
            SelectionAction::Toggle(path) if path == &sources[0][1].path))),
        "highlight-only result must preserve a held raw-row click"
    );
    assert_eq!(window.get_left_rows().row_count(), 4);
    assert_eq!(window.get_left_rows().row_data(1).unwrap().prefix_end, 2);
    assert_eq!(window.get_left_rows().row_data(3).unwrap().prefix_end, 0);
    for (pressed, cursor) in [
        (Key::Home, 1),
        (Key::DownArrow, 2),
        (Key::DownArrow, 2),
        (Key::UpArrow, 1),
        (Key::End, 2),
        (Key::PageUp, 1),
        (Key::PageDown, 2),
    ] {
        key(&window, pressed);
        assert_eq!(window.get_left_cursor(), cursor);
    }
    key(&window, "z");
    let no_match = latest_search(&commands);
    accept(&window, &presenter, &commands, &no_match, &[], 0);
    let count = commands.borrow().len();
    key(&window, Key::Return);
    chord(&window, Key::Alt, Key::Return);
    assert_eq!(
        commands.borrow().len(),
        count,
        "an accepted empty result cannot activate the old cursor"
    );
    assert_eq!(window.get_left_search_count(), 0);
    key(&window, Key::Backspace);
    key(&window, Key::Space);
    assert_eq!(latest_search(&commands).query, "al ");
    key(&window, Key::Backspace);
    assert_eq!(latest_search(&commands).query, "al");
    let navigation_count = commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::Navigate { .. } | Command::Refresh { .. }))
        .count();
    key(&window, Key::Escape);
    assert_eq!(window.get_left_search_query().as_str(), "");
    assert_eq!(
        commands
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::Navigate { .. } | Command::Refresh { .. }))
            .count(),
        navigation_count
    );
    let searches = search_count(&commands);
    chord(&window, Key::Shift, "Z");
    assert_eq!(
        search_count(&commands),
        searches,
        "shifted text cannot start search"
    );
    key(&window, "AL");
    chord(&window, Key::Shift, "!");
    assert_eq!(latest_search(&commands).query, "AL!");
    chord(&window, Key::Control, "a");
    assert_eq!(window.get_left_search_query().as_str(), "AL!");
    key(&window, Key::Escape);

    let searches = search_count(&commands);
    composition(&window, KeyEventType::UpdateComposition, "", "文");
    assert_eq!(window.get_left_search_preedit().as_str(), "文");
    assert_eq!(window.get_left_search_query().as_str(), "");
    assert_eq!(
        search_count(&commands),
        searches,
        "preedit is not a backend query"
    );
    refresh(&window, &presenter.borrow());
    assert_eq!(
        window.get_left_search_preedit().as_str(),
        "文",
        "metadata refresh must retain native preedit"
    );
    composition(&window, KeyEventType::CommitComposition, "文档", "");
    assert_eq!(search_count(&commands), searches + 1);
    assert_eq!(latest_search(&commands).query, "文档");
    composition(&window, KeyEventType::UpdateComposition, "", "未提交");
    window.invoke_activate(1);
    window.invoke_focus_files();
    assert_eq!(window.get_left_search_preedit().as_str(), "");
    assert_eq!(window.get_left_search_query().as_str(), "文档");
    key(&window, "b");
    assert_eq!(latest_search(&commands).pane, PaneId::Right);
    key(&window, Key::Escape);
    key(&window, Key::Tab);
    assert_eq!(window.get_focused_pane(), 0);
    assert_eq!(window.get_left_search_query().as_str(), "文档");
    key(&window, Key::Escape);
    // Slint normalizes an individual key-event payload before delivering it.
    // Separate real commits preserve an existing decomposed query unchanged.
    composition(&window, KeyEventType::CommitComposition, "e\u{301}", "");
    assert_eq!(window.get_left_search_query().as_str(), "é");
    key(&window, Key::Escape);
    for fragments in [vec!["e", "\u{301}"], vec!["👩🏽‍💻"]] {
        for fragment in &fragments {
            composition(&window, KeyEventType::CommitComposition, fragment, "");
        }
        let text = fragments.concat();
        assert_eq!(window.get_left_search_query().as_str(), text);
        assert_eq!(latest_search(&commands).query, text);
        key(&window, Key::Backspace);
        assert_eq!(
            window.get_left_search_query().as_str(),
            "",
            "Backspace must erase an entire original grapheme"
        );
    }
    let searches = search_count(&commands);
    chord(&window, Key::Control, Key::Space);
    assert_eq!(
        window.get_focused_pane(),
        0,
        "Ctrl+Space stays available for IME"
    );
    assert_eq!(search_count(&commands), searches);
    chord(&window, Key::AltGr, "@");
    assert_eq!(window.get_left_search_query().as_str(), "@");
    key(&window, Key::Escape);
    #[cfg(windows)]
    {
        search_keys::remapped_altgr(&window);
        assert_eq!(window.get_left_search_query().as_str(), "@");
        key(&window, Key::Escape);
    }
    let searches = search_count(&commands);
    window.invoke_focus_command();
    key(&window, "echo query");
    assert_eq!(
        search_count(&commands),
        searches,
        "command typing must not start panel search"
    );
    assert!(window.get_command_text().contains("echo query"));
    key(&window, Key::Escape);
    chord(&window, Key::Control, "g");
    key(&window, "folder");
    assert_eq!(
        search_count(&commands),
        searches,
        "path editing must not start panel search"
    );
    window.invoke_focus_files();

    // Preview the actual Options checkbox, then commit and acknowledge its save
    // before continuing in files. Escape now correctly rolls previews back.
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    ui::click_root(&window, "Options", AccessibleRole::Button);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    ui::click_popup(
        &window,
        "Show only filename search matches",
        AccessibleRole::Checkbox,
    );
    assert!(
        window.get_quick_search_filter(),
        "the actual Options checkbox must preview its preference"
    );
    assert!(
        commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. })),
        "the checkbox preview must not persist before OK"
    );
    ui::click_popup(&window, "OK", AccessibleRole::Button);
    let saves = commands
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::SavePreferences {
                revision,
                preferences,
            } => Some((*revision, preferences.quick_search_filter)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(saves.len(), 1, "OK must save the staged preference once");
    assert!(saves[0].1);
    assert!(ui::has_popup(&window), "Options must await the save result");
    presenter.borrow_mut().apply(Event::PreferencesSaved {
        revision: saves[0].0,
        result: Ok(()),
    });
    refresh(&window, &presenter.borrow());
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    assert!(!ui::has_popup(&window));
    assert!(window.get_quick_search_filter());
    window.invoke_focus_files();
    assert_eq!(window.get_focused_pane(), 0);
    key(&window, "al");
    assert_eq!(window.get_focused_pane(), 0);
    let request = latest_search(&commands);
    accept(&window, &presenter, &commands, &request, &[0, 1], 2);
    assert_eq!(window.get_left_rows().row_count(), 3);
    chord(&window, Key::Control, "a");
    assert!(
        commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Selection(request)
                    if matches!(
                        request.action,
                        SelectionAction::Bulk(SelectionOperation::FilesOnly)
                    ) =>
                    request.view.as_ref(),
                _ => None,
            })
            .is_some_and(
                |view| view.by_id.len() == 2 && !view.by_id.contains_key(&sources[0][2].id)
            )
    );
    chord(&window, Key::Shift, Key::LeftArrow);
    chord(&window, Key::Shift, Key::LeftArrow);
    key(&window, "b");
    let request = latest_search(&commands);
    assert_eq!(window.get_left_search_query().as_str(), "b");
    assert_eq!(
        window.get_left_rows().row_count(),
        3,
        "the held press must target a real row in the retained old projection"
    );
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    assert_eq!(window.get_focused_pane(), 0);
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    row_pointer(&window, 1, true);
    assert_eq!(
        window.get_focused_pane(),
        0,
        "a held file-row press keeps its text receiver focused"
    );
    let selections = commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::Selection(_)))
        .count();
    accept(&window, &presenter, &commands, &request, &[2], 1);
    assert_eq!(
        window.get_focused_pane(),
        0,
        "projection replacement cannot take panel input focus"
    );
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    let rebases = commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::Selection(_)))
        .count();
    assert!(rebases >= selections);
    row_pointer(&window, 1, false);
    assert_eq!(
        window.get_focused_pane(),
        0,
        "an invalidated click still leaves the panel ready for typing"
    );
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    assert_eq!(
        commands
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::Selection(_)))
            .count(),
        rebases,
        "a held filtered ordinal must not toggle a different raw entry after projection replacement"
    );
    assert_eq!(
        window.get_left_rows().row_data(1).unwrap().name.as_str(),
        "beta.txt"
    );
    key(&window, "missing");
    let request = latest_search(&commands);
    accept(&window, &presenter, &commands, &request, &[], 0);
    let draft = window.get_command_text();
    chord(&window, Key::Control, Key::Return);
    assert_eq!(
        window.get_command_text(),
        draft,
        "Ctrl+Enter cannot insert a hidden stale basename"
    );
    assert_eq!(window.get_focused_pane(), 0);
    key(&window, Key::Escape);
    assert_eq!(window.get_left_rows().row_count(), 4);
    let drive = root.join("drive");
    presenter.borrow_mut().apply(Event::DesktopContext {
        system_locations: Vec::new(),
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: vec![NavigationLocation {
            id: "native-drive".into(),
            label: "D:".into(),
            path: drive.clone(),
            kind: LocationKind::Volume,
            drive_letter: Some('d'),
        }],
    });
    refresh(&window, &presenter.borrow());
    key(&window, "query");
    let searches = search_count(&commands);
    chord(&window, Key::Shift, "D");
    assert_eq!(
        search_count(&commands),
        searches,
        "an available Shift+drive remains a shortcut during search"
    );
    assert!(
        commands
            .borrow()
            .iter()
            .rev()
            .any(|command| matches!(command,
        Command::Navigate { path, .. } if path == &drive))
    );
    assert_eq!(window.get_left_search_query().as_str(), "");
    composition(&window, KeyEventType::UpdateComposition, "", "旧的组成");
    let tabs = window.get_left_tabs().row_count();
    chord(&window, Key::Control, "t");
    assert_eq!(window.get_left_tabs().row_count(), tabs + 1);
    assert_eq!(
        window.get_left_search_preedit().as_str(),
        "",
        "switching to a new same-path tab must cancel the old native composition"
    );
    assert_eq!(window.get_focused_pane(), 0);
    accept(&window, &presenter, &commands, &request, &[2], 1);
    assert_eq!(window.get_left_search_query().as_str(), "");
    assert_eq!(window.get_left_search_preedit().as_str(), "");
    window.hide().unwrap();
}
