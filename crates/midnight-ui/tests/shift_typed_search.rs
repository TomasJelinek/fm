//! A character typed with Shift (an uppercase letter, "_", "+") starts a filename
//! search in the focused panel like any other typed character, on every platform.
//! Shift+letter switches drives only where a drive has that letter, as on Windows;
//! Linux has no drive letters.
use crate::support::ui;
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

struct Fixture {
    _ui: ui::TestUi,
    window: MainWindow,
    commands: Rc<RefCell<Vec<Command>>>,
}

impl Fixture {
    /// The installed window, both panels listing three files, the left list focused.
    fn new(drives: Vec<NavigationLocation>) -> Self {
        let ui = ui::TestUi::bare();
        let window = ui.window.clone_strong();
        let root = std::env::temp_dir().join("shift typed search");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            root.join("left"),
            root.join("right"),
        )));
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        presenter.borrow_mut().apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::Posix,
            locations: drives,
        });
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        let listings: Vec<_> = commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane,
                    navigation,
                    path,
                    ..
                } => Some((*pane, *navigation, path.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(listings.len(), 2, "both panels are listed at start");
        for (pane, navigation, path) in listings {
            let entries: Vec<Entry> = ["alpha.txt", "Rho.txt", "_draft.txt"]
                .into_iter()
                .enumerate()
                .map(|(row, name)| {
                    Entry::new(
                        EntryId(pane.index() as u64 * 100 + row as u64 + 1),
                        path.join(name),
                        EntryKind::File,
                    )
                })
                .collect();
            presenter.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries: entries.into(),
            });
        }
        refresh(&window, &presenter.borrow());
        window.invoke_focus_files();
        assert_eq!(window.get_focused_pane(), 0);
        commands.borrow_mut().clear();
        Self {
            _ui: ui,
            window,
            commands,
        }
    }

    /// The left panel's filename search: whether it runs, and its text.
    fn search(&self) -> (bool, String) {
        (
            self.window.get_left_search_active(),
            self.window.get_left_search_query().to_string(),
        )
    }

    fn navigations(&self) -> Vec<std::path::PathBuf> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Shift+R types "R" when no drive has that letter (always so on Linux), as do the
/// other characters typed with Shift and an uppercase letter typed with Caps Lock.
#[test]
fn characters_typed_with_shift_start_a_filename_search() {
    let f = Fixture::new(Vec::new());
    for (modifiers, text) in [
        (&[Key::Shift][..], "R"),
        (&[Key::Shift][..], "_"),
        (&[Key::Shift][..], "+"),
        (&[Key::Shift][..], "*"),
        (&[][..], "R"),
    ] {
        let what = format!("{modifiers:?} {text}");
        assert_eq!(f.search(), (false, String::new()), "{what}: no search yet");
        ui::chord(&f.window, modifiers, text);
        assert_eq!(
            f.search(),
            (true, text.to_owned()),
            "{what}: starts a search"
        );
        // A running search takes Shift+letter as text too.
        ui::chord(&f.window, &[Key::Shift], "Q");
        assert_eq!(
            f.search(),
            (true, format!("{text}Q")),
            "{what}: then Shift+Q"
        );
        ui::key(&f.window, Key::Escape);
    }
    assert_eq!(f.window.get_focused_pane(), 0);
    assert!(f.navigations().is_empty(), "no key switched drives");
}

/// Where a drive has the letter, Shift+letter switches the panel to it instead of
/// typing; a letter that no drive has is typed.
#[test]
fn shift_letter_switches_to_a_drive_only_when_one_has_that_letter() {
    let drive = std::env::temp_dir()
        .join("shift typed search")
        .join("Q drive");
    let f = Fixture::new(vec![NavigationLocation {
        id: "q".into(),
        label: "Q:".into(),
        path: drive.clone(),
        kind: LocationKind::Volume,
        drive_letter: Some('Q'),
    }]);
    ui::chord(&f.window, &[Key::Shift], "R");
    assert_eq!(f.search(), (true, "R".to_owned()), "no drive R: typed");
    ui::key(&f.window, Key::Escape);
    assert_eq!(f.search(), (false, String::new()));
    assert!(f.navigations().is_empty());
    ui::chord(&f.window, &[Key::Shift], "Q");
    assert_eq!(f.navigations(), [drive], "Shift+Q opens drive Q");
    assert_eq!(f.search(), (false, String::new()), "and types nothing");
}
