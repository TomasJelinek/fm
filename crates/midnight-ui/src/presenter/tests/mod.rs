use super::*;
use std::{cell::RefCell, io};

mod activation;
mod archive_sizes;
mod archives;
mod bookmarks;
mod bulk_selection;
mod choices;
mod controls;
mod copy_completion;
pub(super) mod copy_dialog;
mod directory_link_tooltip;
mod drag_drop;
mod elevation;
mod file_actions;
mod filename_colors;
mod folder_presence;
mod font_preferences;
mod keyboard;
mod metadata;
mod navigation;
mod navigation_fallback;
mod network_bookmarks;
mod network_neighborhood;
mod operations;
mod panel_navigation;
mod parent_focus;
mod preferences_load;
mod reclamation;
mod search;
mod selected_folder_sizes;
mod selected_navigation;
mod selected_size_status;
mod selection;
mod selection_rectangle;
mod shortcuts;
mod size_continuity;
mod tabs;
mod tree;
mod tree_checksums;
mod tree_clipboard;
mod tree_comparison;
mod tree_copy;
mod tree_delete;
mod tree_drag;
mod tree_editor;
mod tree_refresh;
mod tree_shell;
mod tree_user_commands;
mod user_commands;
mod volumes;
mod watch;

/// The first successful settings read also requests the comparator image metadata
/// (`Command::ListComparatorCapabilities`). Senders that inspect startup saves
/// accept that request and check every save.
fn startup_metadata(command: &Command) -> bool {
    matches!(command, Command::ListComparatorCapabilities { .. })
}

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
    let sent = RefCell::new(Vec::new());
    assert!(p.navigate(pane, path.into(), None, &|c| {
        sent.borrow_mut().push(c);
        true
    }));
    sent.into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Navigate { navigation, .. } | Command::Refresh { navigation, .. } => {
                Some(navigation)
            }
            _ => None,
        })
        .expect("navigation must send Navigate or Refresh")
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

/// A late reply for a released owner must neither reach another owner nor change
/// the visible notice/status, and its payload must leave the UI thread retired.
pub(super) fn assert_late_event_is_only_retired(
    p: &mut Presenter,
    late: impl FnOnce(&mut Presenter),
) {
    drop(p.take_retired());
    let notice = p.desktop.notice_generation;
    let status = p.desktop.status.clone();
    late(p);
    assert_eq!(
        p.take_retired().len(),
        1,
        "the late payload is retired, not consumed or dropped on the UI thread"
    );
    assert_eq!(p.desktop.notice_generation, notice);
    assert_eq!(p.desktop.status, status);
}

/// The provider's last owner must leave the UI thread through retirement: nothing
/// was dropped yet, and dropping the retired values on a worker drops it once.
fn reclaim(p: &mut Presenter, drops: &std::sync::atomic::AtomicUsize) {
    use std::sync::atomic::Ordering;
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "the provider's last owner was dropped on the UI thread"
    );
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
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
        deferred: Default::default(),
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

mod thumbnail_policy;
