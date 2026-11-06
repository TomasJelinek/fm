use super::*;
use std::{cell::RefCell, io};

mod activation;
mod bookmarks;
mod bulk_selection;
mod choices;
mod controls;
mod drag_drop;
mod file_actions;
mod font_preferences;
mod keyboard;
mod metadata;
mod navigation;
mod operations;
mod panel_navigation;
mod preferences_load;
mod reclamation;
mod search;
mod selection;
mod shortcuts;
mod size_continuity;
mod tabs;
mod volumes;
mod watch;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
use unix::test_root;
#[cfg(windows)]
use windows::test_root;

fn entry(id: u64, name: &str) -> Entry {
    Entry::new(EntryId(id), PathBuf::from(name), EntryKind::Directory)
}
fn begin(p: &mut Presenter, pane: PaneId, path: &str) -> NavigationId {
    let sent = RefCell::new(None);
    assert!(p.navigate(pane, path.into(), None, &|c| {
        *sent.borrow_mut() = Some(c);
        true
    }));
    match sent.into_inner().unwrap() {
        Command::Navigate { navigation, .. } | Command::Refresh { navigation, .. } => navigation,
        _ => panic!(),
    }
}
fn finish(p: &mut Presenter, pane: PaneId, id: NavigationId, entries: Vec<Entry>) {
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    p.apply(Event::ListingFinished {
        pane,
        navigation: id,
        entries: entries.into(),
        index,
    });
}

/// Explicit small snapshots keep legacy cursor/navigation tests independent
/// of asynchronous delivery; the engine tests exercise the real worker.
fn selected_ids(p: &Presenter, pane: PaneId) -> HashSet<EntryId> {
    p.panes[pane.index()]
        .rows()
        .iter()
        .filter(|entry| p.panes[pane.index()].selected.contains_path(&entry.path))
        .map(|entry| entry.id)
        .collect()
}

fn set_selected(p: &mut Presenter, pane: PaneId, ids: &[EntryId]) {
    let rows = p.panes[pane.index()].rows();
    let basis = Arc::new(DirectoryIndex::from_entries(rows));
    let mut bits = vec![0; rows.len().div_ceil(64)];
    for id in ids {
        let index = basis.by_id[id];
        bits[index / 64] |= 1 << (index % 64);
    }
    p.panes[pane.index()].selected = Arc::new(SelectionSnapshot::from_bits(basis, bits).unwrap());
}

fn fixture_toggle(p: &mut Presenter, pane: PaneId, command: Option<Command>) {
    let Some(Command::Selection(request)) = command else {
        return;
    };
    let SelectionAction::Toggle(path) = &request.action else {
        panic!("single-entry input must send a raw-path toggle")
    };
    let rows = p.panes[pane.index()].rows();
    let basis = Arc::new(DirectoryIndex::from_entries(rows));
    let mut bits = vec![0; rows.len().div_ceil(64)];
    for (index, entry) in rows.iter().enumerate() {
        let selected = p.panes[pane.index()].selected.contains_path(&entry.path);
        if selected != (&entry.path == path) {
            bits[index / 64] |= 1 << (index % 64);
        }
    }
    p.apply(Event::SelectionChanged {
        pane,
        navigation: request.navigation,
        revision: request.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(basis, bits).unwrap()),
        deferred: false,
    });
}

fn select_row(p: &mut Presenter, pane: PaneId, row: usize, toggle: bool) {
    let sent = RefCell::new(None);
    p.select_row(pane, row, toggle, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    fixture_toggle(p, pane, sent.into_inner());
}

fn move_cursor(p: &mut Presenter, pane: PaneId, key: i32) {
    let sent = RefCell::new(None);
    p.move_cursor(pane, key, &|command| {
        if matches!(command, Command::Selection(_)) {
            *sent.borrow_mut() = Some(command);
        }
        true
    });
    fixture_toggle(p, pane, sent.into_inner());
}
mod selection_feedback;
