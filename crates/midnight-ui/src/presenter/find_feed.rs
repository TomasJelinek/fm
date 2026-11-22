//! Feed to panel: a Find's results as a temporary list in a panel, a virtual
//! "search results" location over the panel's own folder. Viewing, editing,
//! copying, moving, deleting and the clipboard act on the real native files
//! through the ordinary backend flows; nothing can be created or renamed in the
//! list. Leaving it (or any navigation of that panel) shows the folder again.
//! The folder under the list is never acted on: every entry point that would
//! act on a panel's folder asks `feed_refuses` (or routes to the list) first.
use super::*;

#[cfg(test)]
#[path = "find_feed_tests.rs"]
mod tests;

/// The title of an action a list of search results refuses.
pub(crate) const REFUSED: &str = "Not available in search results";

/// Why `action` is refused while its panel shows search results.
pub(crate) fn refusal(action: &str) -> String {
    format!(
        "{action} is not available in a list of search results. Leave the list with Backspace to use it in the folder."
    )
}

/// The results one panel shows instead of its folder.
pub(crate) struct FeedPane {
    pub(crate) entries: Arc<[Entry]>,
    pub(crate) index: Arc<DirectoryIndex>,
    pub(crate) selected: Vec<bool>,
    pub(crate) selected_count: usize,
    pub(crate) selected_bytes: u64,
    pub(crate) cursor: usize,
    /// Rows a page key moves; the list reports what it shows.
    pub(crate) page: usize,
    /// What the Find searched for, shown as the list's location.
    pub(crate) title: String,
    /// Results that cannot be shown here: archive members.
    pub(crate) omitted: usize,
    /// The panel's location when it was fed; showing another location leaves the list.
    pub(crate) location: String,
    /// Changes whenever rows, marks or the cursor change.
    pub(crate) revision: u64,
    /// Unique to these rows: a new feed or rows a delete removed get a new value.
    pub(crate) rows_serial: u64,
    /// A delete started from the list: its operation and targets.
    deleting: Option<(OperationId, Vec<PathBuf>)>,
}

/// A new value for `FeedPane::rows_serial`, never reused within the process.
fn next_rows_serial() -> u64 {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl FeedPane {
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn focused(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    fn set_mark(&mut self, row: usize, mark: bool) {
        let Some(old) = self.selected.get_mut(row) else {
            return;
        };
        if *old == mark {
            return;
        }
        *old = mark;
        let size = self.entries[row]
            .metadata
            .size
            .filter(|_| self.entries[row].kind == EntryKind::File)
            .unwrap_or(0);
        if mark {
            self.selected_count += 1;
            self.selected_bytes = self.selected_bytes.saturating_add(size);
        } else {
            self.selected_count -= 1;
            self.selected_bytes = self.selected_bytes.saturating_sub(size);
        }
    }

    /// The selected rows, or the focused one when nothing is selected.
    fn targets(&self) -> Vec<&Entry> {
        if self.selected_count > 0 {
            self.entries
                .iter()
                .zip(&self.selected)
                .filter_map(|(entry, selected)| selected.then_some(entry))
                .collect()
        } else {
            self.focused().into_iter().collect()
        }
    }

    /// The list as a captured selection for the backend's path-based flows.
    fn files(&self) -> Option<SelectedFiles> {
        let mut bits = vec![0u64; self.entries.len().div_ceil(64)];
        for (entry, selected) in self.entries.iter().zip(&self.selected) {
            if *selected {
                let row = *self.index.by_id.get(&entry.id)?;
                bits[row / 64] |= 1 << (row % 64);
            }
        }
        let selection = SelectionSnapshot::from_bits(self.index.clone(), bits).ok()?;
        let fallback = selection
            .is_empty()
            .then(|| self.focused().map(|entry| entry.path.clone()))
            .flatten();
        if selection.is_empty() && fallback.is_none() {
            return None;
        }
        Some(SelectedFiles {
            selection: Arc::new(selection),
            view: self.index.clone(),
            fallback,
        })
    }
}

/// Cursor keys inside the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FeedMove {
    Up,
    Down,
    PageUp,
    PageDown,
    First,
    Last,
    PreviousSelected,
    NextSelected,
}

/// What a key or the function bar asks of a panel that shows search results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FeedAction {
    Leave,
    GoTo,
    Move(FeedMove),
    /// Insert marks and moves down; Space marks in place.
    Toggle {
        advance: bool,
    },
    /// A bulk selection key, with the panels' row policy (`SelectionOperation::selects`):
    /// Ctrl+A marks the files only and Ctrl+I inverts the files, both leaving folders
    /// as a panel does.
    Bulk(SelectionOperation),
    /// Keypad *: inverts as in a panel (`Presenter::mask_invert_operation`), the
    /// folders too when Select by mask includes them.
    MaskInvert,
    View(bool),
    Edit,
    Transfer(TransferIntent),
    Delete(DeleteMode),
    Clipboard {
        cut: bool,
    },
    Properties,
}

impl Presenter {
    /// Feed to panel: the session's native results, in the Find window's order,
    /// replace the folder of the panel that opened the Find. Returns that panel.
    pub(crate) fn feed_find_results(&mut self, id: FindSessionId) -> Option<PaneId> {
        let s = self.finds.sessions.get(&id)?;
        if s.running {
            if let Some(s) = self.finds.session_mut(&id) {
                s.error = "Wait for the search to finish, or stop it, before feeding its results to a panel.".into();
            }
            return None;
        }
        let ordered: Vec<&Arc<FindResult>> = match &s.order {
            Some(order) if order.ids.len() == s.results.len() => order
                .ids
                .iter()
                .filter_map(|id| s.results.get(usize::try_from(id.0.checked_sub(1)?).ok()?))
                .collect(),
            _ => s.results.iter().collect(),
        };
        // The Find's selection arrives as the list's marks, unless it only follows the
        // cursor; the list's cursor starts on the Find's cursor result.
        let marked = s.selection_is_explicit();
        let cursor_slot = s.selection.cursor();
        let mut seen: HashMap<PathBuf, usize> = HashMap::new();
        let mut entries = Vec::with_capacity(ordered.len());
        let mut marks: Vec<usize> = Vec::new();
        let mut cursor = 0;
        let mut omitted = 0;
        for result in ordered {
            if result.archive.is_some()
                || !matches!(result.entry.kind, EntryKind::File | EntryKind::Directory)
            {
                omitted += 1;
                continue;
            }
            let row = *seen
                .entry(result.entry.path.clone())
                .or_insert(entries.len());
            let slot = result_selection::slot(result.id);
            if marked && slot.is_some_and(|slot| s.selection.is_selected(slot)) {
                marks.push(row);
            }
            if slot.is_some() && slot == cursor_slot {
                cursor = row;
            }
            if row < entries.len() {
                continue;
            }
            let mut entry = Entry::new(
                EntryId(entries.len() as u64 + 1),
                result.entry.path.clone(),
                result.entry.kind,
            );
            entry.metadata = result.entry.metadata.clone();
            entries.push(entry);
        }
        let pane = s.origin;
        if entries.is_empty() {
            if let Some(s) = self.finds.session_mut(&id) {
                s.error = if omitted > 0 {
                    "Only archive members were found; open them with Go to file.".into()
                } else {
                    "There are no results to feed to a panel.".into()
                };
            }
            return None;
        }
        let roots: Vec<_> = s
            .roots
            .iter()
            .filter(|root| root.searched)
            .map(|root| root.path.to_string_lossy().into_owned())
            .collect();
        let title = format!(
            "Search results ({}) · {}",
            self.number_locale.format_u64(entries.len() as u64),
            roots.join("; ")
        );
        let count = entries.len();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let location = self.panes[pane.index()].display_location_text();
        let mut feed = FeedPane {
            entries: entries.into(),
            index,
            selected: vec![false; count],
            selected_count: 0,
            selected_bytes: 0,
            cursor,
            page: match self.finds.feed_pages[pane.index()] {
                0 => 20,
                rows => rows,
            },
            title,
            omitted,
            location,
            revision: 0,
            rows_serial: next_rows_serial(),
            deleting: None,
        };
        for row in marks {
            feed.set_mark(row, true);
        }
        self.retired
            .push(Box::new(self.finds.feeds[pane.index()].replace(feed)));
        self.active = pane;
        Some(pane)
    }

    /// The results the panel shows, while it still shows the location it was fed in.
    pub(crate) fn feed(&self, pane: PaneId) -> Option<&FeedPane> {
        self.finds.feeds[pane.index()]
            .as_ref()
            .filter(|feed| feed.location == self.panes[pane.index()].display_location_text())
    }

    fn feed_mut(&mut self, pane: PaneId) -> Option<&mut FeedPane> {
        let location = self.panes[pane.index()].display_location_text();
        self.finds.feeds[pane.index()]
            .as_mut()
            .filter(|feed| feed.location == location)
    }

    /// Leave the list: the panel shows its folder again.
    pub(crate) fn feed_close(&mut self, pane: PaneId) {
        self.retired
            .push(Box::new(self.finds.feeds[pane.index()].take()));
    }

    /// The list reports how many rows it shows, for the page keys.
    pub(crate) fn feed_page_rows(&mut self, pane: PaneId, rows: usize) {
        let rows = rows.max(1);
        self.finds.feed_pages[pane.index()] = rows;
        if let Some(feed) = self.feed_mut(pane) {
            feed.page = rows;
        }
    }

    /// A click: plain moves the cursor; Ctrl marks the row; Shift marks the range
    /// from the cursor.
    pub(crate) fn feed_click(&mut self, pane: PaneId, row: usize, control: bool, shift: bool) {
        self.active = pane;
        let Some(feed) = self.feed_mut(pane) else {
            return;
        };
        if row >= feed.entries.len() {
            return;
        }
        if shift {
            let (first, last) = (feed.cursor.min(row), feed.cursor.max(row));
            for index in first..=last {
                feed.set_mark(index, true);
            }
        } else if control {
            let mark = !feed.selected[row];
            feed.set_mark(row, mark);
        }
        feed.cursor = row;
        feed.changed();
    }

    pub(crate) fn feed_action(
        &mut self,
        pane: PaneId,
        action: FeedAction,
        send: &dyn Fn(Command) -> bool,
    ) {
        use self::{FeedAction as A, FeedMove as M};
        if self.feed(pane).is_none() {
            return;
        }
        self.active = pane;
        match action {
            A::Leave => self.feed_close(pane),
            A::GoTo => self.feed_go_to(pane, send),
            A::Move(step) => {
                let feed = self.feed_mut(pane).expect("shown feed");
                let last = feed.entries.len().saturating_sub(1);
                let page = feed.page.max(1);
                feed.cursor = match step {
                    M::Up => feed.cursor.saturating_sub(1),
                    M::Down => (feed.cursor + 1).min(last),
                    M::PageUp => feed.cursor.saturating_sub(page),
                    M::PageDown => (feed.cursor + page).min(last),
                    M::First => 0,
                    M::Last => last,
                    M::PreviousSelected => (0..feed.cursor)
                        .rev()
                        .find(|row| feed.selected[*row])
                        .unwrap_or(feed.cursor),
                    M::NextSelected => (feed.cursor + 1..feed.entries.len())
                        .find(|row| feed.selected[*row])
                        .unwrap_or(feed.cursor),
                };
                feed.changed();
            }
            A::Toggle { advance } => {
                let feed = self.feed_mut(pane).expect("shown feed");
                if feed.entries.is_empty() {
                    return;
                }
                let row = feed.cursor;
                let mark = !feed.selected[row];
                feed.set_mark(row, mark);
                if advance {
                    feed.cursor = (row + 1).min(feed.entries.len().saturating_sub(1));
                }
                feed.changed();
            }
            A::Bulk(operation) => self.feed_bulk(pane, operation),
            A::MaskInvert => self.feed_bulk(pane, self.mask_invert_operation()),
            A::View(basic) => self.feed_view(pane, basic),
            A::Edit => self.feed_edit(pane, send),
            A::Transfer(intent) => self.feed_transfer(pane, intent),
            A::Delete(mode) => self.feed_delete(pane, mode, send),
            A::Clipboard { cut } => self.feed_clipboard(pane, cut, send),
            A::Properties => self.feed_properties(pane, send),
        }
    }

    /// Mark or unmark every row as `operation` does in a panel.
    fn feed_bulk(&mut self, pane: PaneId, operation: SelectionOperation) {
        let Some(feed) = self.feed_mut(pane) else {
            return;
        };
        for row in 0..feed.entries.len() {
            let directory = matches!(
                feed.entries[row].kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            );
            let mark = operation.selects(directory, feed.selected[row]);
            feed.set_mark(row, mark);
        }
        feed.changed();
    }

    /// An action the list cannot offer, explained instead of acting on the folder
    /// hidden under the list.
    pub(crate) fn feed_refuse(&mut self, action: &str) {
        self.desktop.report(REFUSED, refusal(action));
    }

    /// The guard of the paths that act on a panel's folder (toolbar, menus,
    /// command line, custom commands): while the panel shows search results,
    /// that folder is hidden under the list, so `action` is refused with the
    /// list's explanation. Returns whether it was refused.
    pub(crate) fn feed_refuses(&mut self, pane: PaneId, action: &str) -> bool {
        if self.feed(pane).is_none() {
            return false;
        }
        self.feed_refuse(action);
        true
    }

    /// A change of files asked of a panel that shows search results: Delete
    /// deletes the marked results, as F8 in the list does; the others would act
    /// on the folder under the list and are refused.
    pub(crate) fn feed_mutation(
        &mut self,
        pane: PaneId,
        kind: MutationKind,
        send: &dyn Fn(Command) -> bool,
    ) {
        if let MutationKind::Delete(mode) = kind {
            self.feed_action(pane, FeedAction::Delete(mode), send);
        } else {
            self.feed_refuse(operations::mutation_action(kind));
        }
    }

    /// Enter: the panel goes to the focused result, leaving the list. A folder is
    /// opened; a file is shown in its folder with the cursor on it.
    fn feed_go_to(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let Some(entry) = self.feed(pane).and_then(|feed| feed.focused()).cloned() else {
            return;
        };
        let (target, wanted) = if entry.kind == EntryKind::Directory {
            (entry.path.clone(), None)
        } else {
            let Some(parent) = entry.path.parent() else {
                return;
            };
            (parent.to_owned(), Some(entry.path.clone()))
        };
        self.feed_close(pane);
        if self.navigate(pane, target, None, send) {
            if let Some(pending) = &mut self.panes[pane.index()].pending {
                pending.wanted_cursor = wanted;
            }
        } else {
            self.desktop.report(
                "Go to file",
                "Panel navigation queue is busy. Try again.".into(),
            );
        }
    }

    fn feed_view(&mut self, pane: PaneId, basic: bool) {
        let Some(feed) = self.feed(pane) else {
            return;
        };
        let Some(entry) = feed.focused().filter(|entry| entry.kind == EntryKind::File) else {
            self.desktop.report(
                "View",
                "Focus a file in the search results to view it.".into(),
            );
            return;
        };
        let Some(disabled_plugins) = self.viewer_plugins.startup_disabled.clone() else {
            let reason = self.viewer_plugins.policy_unavailable().to_owned();
            self.desktop.report("Cannot open viewer", reason);
            return;
        };
        // The viewer walks the results with Space and Backspace, like a folder.
        let launch = ViewerLaunch {
            find: None,
            associations: self.viewer_plugins.startup_associations.clone(),
            tree: None,
            generated: None,
            preferences: Arc::new(self.preferences.values.viewer.clone()),
            archive: None,
            entries: feed.entries.clone(),
            view: feed.index.clone(),
            focused: entry.path.clone(),
            basic,
            disabled_plugins,
        };
        self.retired
            .push(Box::new(self.finds.feed_viewer.replace(launch)));
    }

    fn feed_edit(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let Some(path) = self
            .feed(pane)
            .and_then(|feed| feed.focused())
            .filter(|entry| entry.kind == EntryKind::File)
            .map(|entry| entry.path.clone())
        else {
            self.desktop.report(
                "Edit",
                "Focus a file in the search results to edit it.".into(),
            );
            return;
        };
        if !self.editor_preferences_ready() {
            self.desktop.report("Cannot open editor", "Saved editor settings are not available. Wait for settings to load, or resolve the settings error before editing.".into());
            return;
        }
        let request = self.desktop.request();
        if send(Command::EditFile {
            request,
            path,
            editor: self.preferences.values.editor.clone(),
        }) {
            self.desktop.requests.insert(request);
            self.desktop.status = "Opening external editor…".into();
        } else {
            self.desktop.report(
                "Editor queue busy",
                "Try F4 again when the backend is ready.".into(),
            );
        }
    }

    /// F5/F6: the ordinary Copy/Move dialog, to the other panel's folder.
    fn feed_transfer(&mut self, pane: PaneId, intent: TransferIntent) {
        if self.operations.closing || self.copies.draft.is_some() {
            return;
        }
        if !self.owned_command_scope_available() {
            self.desktop.report(
                "Copy/Move queue busy",
                "Wait for the previous transfer request to finish. Try again.".into(),
            );
            return;
        }
        if !self.native_action_allowed(pane.other()) {
            return;
        }
        let Some(feed) = self.feed(pane) else {
            return;
        };
        let Some(files) = feed.files() else {
            return;
        };
        let items = match feed.focused() {
            Some(entry) if files.selection.is_empty() => {
                crate::presenter::copy_dialog::DraftItems::entry(entry)
            }
            _ => crate::presenter::copy_dialog::DraftItems::selected(&files),
        };
        let destination = self.panes[pane.other().index()].display_path().to_owned();
        let request = self.desktop.request();
        self.copies.draft = Some(crate::presenter::copy_dialog::CopyDraft {
            token: format!("copy-draft:{}", request.0),
            input: crate::presenter::copy_dialog::CopyDraftInput::Native(files),
            source: "Search results".into(),
            destination,
            items,
            error: String::new(),
            intent,
            retire_requested: false,
        });
    }

    /// F8/Shift+F8: the ordinary Delete confirmation and progress. Each target must
    /// still be the file or folder the search found.
    fn feed_delete(&mut self, pane: PaneId, mode: DeleteMode, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing {
            return;
        }
        let Some(feed) = self.feed(pane) else {
            return;
        };
        if feed.deleting.is_some() {
            self.desktop.report(
                "Delete",
                "A delete from these search results is still running.".into(),
            );
            return;
        }
        let targets: Vec<CapturedPath> = feed
            .targets()
            .into_iter()
            .map(|entry| CapturedPath {
                path: entry.path.clone(),
                kind: entry.kind,
                size: entry.metadata.size,
            })
            .collect();
        let Some(serial) = self.finds.feed_serial.checked_add(1) else {
            return;
        };
        let sources = match CapturedPathsDelete::new(pane, serial, mode, targets) {
            Ok(sources) => Arc::new(sources),
            Err(message) => {
                self.desktop.report("Delete", message.into());
                return;
            }
        };
        let paths: Vec<PathBuf> = sources
            .targets()
            .iter()
            .map(|target| target.path.clone())
            .collect();
        let Some(id) = self.next_operation_id(send) else {
            self.retired.push(Box::new(sources));
            return;
        };
        let source_directory = self.panes[pane.index()].path.clone();
        // Keep an owner for worker reclamation even if the sender drops a refused command.
        self.retired.push(Box::new(sources.clone()));
        if send(Command::PrepareCapturedDelete(PrepareCapturedDelete {
            id,
            sources,
            source_directory: source_directory.clone(),
        })) {
            self.finds.feed_serial = serial;
            self.register_preparing_operation(id, MutationKind::Delete(mode), source_directory);
            if let Some(feed) = self.feed_mut(pane) {
                feed.deleting = Some((id, paths));
            }
        } else {
            self.desktop
                .report("Operation queue busy", "Try again.".into());
        }
    }

    fn feed_clipboard(&mut self, pane: PaneId, cut: bool, send: &dyn Fn(Command) -> bool) {
        let Some(files) = self.feed(pane).and_then(FeedPane::files) else {
            return;
        };
        let request = self.desktop.request();
        let command = if cut {
            Command::CutFiles { request, files }
        } else {
            Command::CopyFiles { request, files }
        };
        if send(command) {
            self.desktop.requests.insert(request);
        } else {
            self.desktop
                .report("Clipboard queue busy", "Try the action again.".into());
        }
    }

    fn feed_properties(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let Some(files) = self.feed(pane).and_then(FeedPane::files) else {
            return;
        };
        let request = self.desktop.request();
        // Stops the folder count the summary continues with, as for panel rows.
        let cancel = Cancellation::default();
        if send(Command::ShowSelectionProperties {
            request,
            selection: files.selection,
            view: files.view,
            fallback: files.fallback,
            cancel: cancel.clone(),
        }) {
            self.desktop.requests.insert(request);
            self.properties_requested(request, cancel);
        } else {
            self.desktop
                .report("Properties queue busy", "Try the action again.".into());
        }
    }

    /// After an event: forget lists whose panel moved on, and drop the rows a
    /// completed delete removed.
    pub(super) fn flush_feeds(&mut self) {
        for slot in 0..2 {
            let location = self.panes[slot].display_location_text();
            if self.finds.feeds[slot]
                .as_ref()
                .is_some_and(|feed| feed.location != location)
            {
                self.retired.push(Box::new(self.finds.feeds[slot].take()));
                continue;
            }
            let Some(feed) = &mut self.finds.feeds[slot] else {
                continue;
            };
            let Some((id, _)) = &feed.deleting else {
                continue;
            };
            let state = self
                .operations
                .progress
                .iter()
                .find(|progress| progress.id == *id)
                .map(|progress| (progress.state, progress.failed));
            match state {
                Some((state, _)) if !state.is_terminal() => {}
                Some((OperationState::Completed, 0)) => {
                    let (_, paths) = feed.deleting.take().expect("matched delete");
                    let gone: HashSet<PathBuf> = paths.into_iter().collect();
                    // A row went with a deleted path or a deleted folder above it:
                    // one lookup per path component, not one scan of the deleted
                    // paths per row.
                    let keep: Vec<usize> = (0..feed.entries.len())
                        .filter(|row| {
                            !feed.entries[*row]
                                .path
                                .ancestors()
                                .any(|path| gone.contains(path))
                        })
                        .collect();
                    let entries: Vec<Entry> =
                        keep.iter().map(|row| feed.entries[*row].clone()).collect();
                    let selected: Vec<bool> = keep.iter().map(|row| feed.selected[*row]).collect();
                    let cursor = keep
                        .iter()
                        .position(|row| *row >= feed.cursor)
                        .unwrap_or(keep.len().saturating_sub(1));
                    let index = Arc::new(DirectoryIndex::from_entries(&entries));
                    self.retired
                        .push(Box::new(mem::replace(&mut feed.entries, entries.into())));
                    self.retired
                        .push(Box::new(mem::replace(&mut feed.index, index)));
                    feed.selected = selected;
                    feed.selected_count = 0;
                    feed.selected_bytes = 0;
                    for row in 0..feed.selected.len() {
                        if mem::take(&mut feed.selected[row]) {
                            feed.set_mark(row, true);
                        }
                    }
                    feed.cursor = cursor;
                    feed.rows_serial = next_rows_serial();
                    feed.changed();
                }
                // Cancelled, failed or forgotten: the rows stay; acting on a
                // removed file reports that it is gone.
                _ => {
                    feed.deleting = None;
                }
            }
        }
    }
}
