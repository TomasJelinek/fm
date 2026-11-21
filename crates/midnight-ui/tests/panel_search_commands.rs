//! A filename search typed into the focused panel never stops a command. Key events go
//! through the panel's search input, as typing does: commands close the search and act
//! on the focused row, and Enter opens only a match.
use crate::support::ui;

use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{ComponentHandle, SharedString, platform::Key};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

const NAMES: [&str; 3] = ["alpha.txt", "beta.txt", "readme.md"];

/// What F3 and Alt+F3 returned, as the application's viewer handler receives it.
type Viewed = Result<PathBuf, Option<String>>;

struct Fixture {
    _ui: ui::TestUi,
    window: MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    root: PathBuf,
    views: Rc<RefCell<Vec<Viewed>>>,
}

fn fixture() -> Fixture {
    let ui = ui::TestUi::bare();
    let window = ui.window.clone_strong();
    let root = std::env::temp_dir().join("midnight-search-commands 文档");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        captured.borrow_mut().push(command);
        true
    });
    install(&window, presenter.clone(), send.clone());
    let views = Rc::new(RefCell::new(Vec::new()));
    let (state, viewed, weak) = (presenter.clone(), views.clone(), window.as_weak());
    window.on_view_file(move |pane, basic| {
        let pane = if pane == 0 {
            PaneId::Left
        } else {
            PaneId::Right
        };
        let launch = state.borrow_mut().view_command(pane, basic, send.as_ref());
        viewed
            .borrow_mut()
            .push(launch.map(|launch| launch.focused));
        if let Some(window) = weak.upgrade() {
            refresh(&window, &state.borrow());
        }
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
            .expect("each pane lists its folder");
        let entries: Arc<[Entry]> = NAMES
            .iter()
            .enumerate()
            .map(|(row, name)| {
                Entry::new(
                    EntryId((pane.index() * 100 + row + 1) as u64),
                    root.join(name),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    refresh(&window, &presenter.borrow());
    window.invoke_focus_files();
    // The cursor on readme.md, the last row.
    key(&window, Key::End);
    Fixture {
        _ui: ui,
        window,
        presenter,
        commands,
        root,
        views,
    }
}

fn key(window: &MainWindow, text: impl Into<SharedString>) {
    ui::platform_keys::key(window, text.into());
}

fn type_text(window: &MainWindow, text: &str) {
    for character in text.chars() {
        key(window, character.to_string());
    }
}

impl Fixture {
    fn readme(&self) -> PathBuf {
        self.root.join("readme.md")
    }

    fn latest_search(&self) -> SearchRequest {
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Search(request) => Some(request.clone()),
                _ => None,
            })
            .expect("typing searches on the backend")
    }

    /// The backend's answer to `request`: the rows at `matches` start with its text.
    fn answer(&self, request: &SearchRequest, matches: &[usize]) {
        let matched: Vec<_> = matches
            .iter()
            .map(|row| request.source[*row].clone())
            .collect();
        let rows = |rows: &mut dyn Iterator<Item = usize>, end| -> Arc<[SearchMatch]> {
            rows.map(|source_row| SearchMatch {
                source_row,
                display_prefix_end: end,
            })
            .collect::<Vec<_>>()
            .into()
        };
        self.presenter.borrow_mut().apply_with_sender(
            Event::SearchReady {
                pane: request.pane,
                navigation: request.navigation,
                revision: request.revision,
                result: Arc::new(SearchResult {
                    source: request.source.clone(),
                    filtered: rows(&mut (0..request.source.len()), 0),
                    filtered_index: Arc::new(DirectoryIndex::from_entries(&request.source)),
                    matches: rows(&mut matches.iter().copied(), request.query.len()),
                    index: Arc::new(DirectoryIndex::from_entries(&matched)),
                }),
            },
            &|command| {
                self.commands.borrow_mut().push(command);
                true
            },
        );
        refresh(&self.window, &self.presenter.borrow());
    }

    /// Text typed by accident: no name matches it.
    fn type_unmatched(&self) -> SearchRequest {
        type_text(&self.window, "zz");
        let request = self.latest_search();
        assert_eq!(request.query, "zz");
        self.answer(&request, &[]);
        assert_eq!(self.window.get_left_search_count(), 0);
        request
    }

    fn opened(&self) -> Vec<PathBuf> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::OpenEntry { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn f3_with_a_nonmatching_search_views_the_focused_file_and_closes_the_search() {
    let f = fixture();
    let cursor = f.window.get_left_cursor();
    f.type_unmatched();
    assert_eq!(f.window.get_left_search_query().as_str(), "zz");
    key(&f.window, Key::F3);
    assert_eq!(f.views.borrow().as_slice(), [Viewed::Ok(f.readme())]);
    assert_eq!(
        f.window.get_left_search_query().as_str(),
        "",
        "the search closed"
    );
    assert_eq!(f.window.get_left_cursor(), cursor, "the cursor stayed");
    assert_eq!(f.window.get_left_watch_status().as_str(), "");
    // Alt+F3 the same way.
    f.type_unmatched();
    ui::chord(&f.window, &[Key::Alt], Key::F3);
    assert_eq!(f.views.borrow().last(), Some(&Viewed::Ok(f.readme())));
    assert_eq!(f.window.get_left_search_query().as_str(), "");
}

#[test]
fn enter_with_a_nonmatching_search_opens_nothing_and_says_so() {
    let f = fixture();
    let cursor = f.window.get_left_cursor();
    f.type_unmatched();
    key(&f.window, Key::Return);
    assert!(f.opened().is_empty(), "Enter starts nothing");
    assert_eq!(
        f.window.get_left_search_query().as_str(),
        "",
        "the search closed"
    );
    assert_eq!(f.window.get_left_cursor(), cursor);
    assert_eq!(
        f.window.get_left_watch_status().as_str(),
        "No name matches “zz”"
    );
    // The panel now shows readme.md focused without a search: Enter opens it.
    key(&f.window, Key::Return);
    assert_eq!(f.opened(), [f.readme()]);
    assert_eq!(f.window.get_left_watch_status().as_str(), "");
    // Typing a match and Enter opens the match, as before.
    type_text(&f.window, "bet");
    let request = f.latest_search();
    f.answer(&request, &[1]);
    key(&f.window, Key::Return);
    assert_eq!(f.opened(), [f.readme(), f.root.join("beta.txt")]);
}

#[test]
fn commands_cancel_a_computing_search_and_act_on_the_focused_row() {
    let f = fixture();
    let cursor = f.window.get_left_cursor();
    type_text(&f.window, "al");
    let request = f.latest_search();
    // Alt+Enter: Properties of the row focused when it was pressed.
    ui::chord(&f.window, &[Key::Alt], Key::Return);
    assert!(
        f.commands.borrow().iter().any(|command| matches!(command,
            Command::ShowSelectionProperties { fallback: Some(path), .. } if *path == f.readme())),
        "Properties of the focused file"
    );
    assert_eq!(f.window.get_left_search_query().as_str(), "");
    // The closed search's late result does not move the cursor to "alpha.txt".
    f.answer(&request, &[0]);
    assert_eq!(f.window.get_left_cursor(), cursor);
    // Delete with a computing search deletes the focused row.
    type_text(&f.window, "b");
    key(&f.window, Key::Delete);
    let prepared = f
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PrepareOperation(request) => Some((request.kind, request.focused)),
            _ => None,
        })
        .expect("Delete prepares the deletion");
    assert_eq!(
        prepared,
        (MutationKind::Delete(DeleteMode::Trash), Some(EntryId(3)))
    );
    assert_eq!(f.window.get_left_search_query().as_str(), "");
}
