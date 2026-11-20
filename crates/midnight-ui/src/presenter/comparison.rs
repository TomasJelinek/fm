//! Atomic publication of two immutable, revision-checked selection snapshots.
use super::*;
#[cfg(test)]
#[path = "comparison_tests.rs"]
mod tests;
mod tree;

#[derive(Default)]
pub(crate) struct ComparisonState {
    pub(crate) open: bool,
    pub(crate) busy: bool,
    pub(crate) error: String,
    pub(crate) summary: String,
    pub(crate) options: CompareOptions,
    pub(super) seeds: [Option<Arc<AuthoritativeSelection>>; 2],
    next: u64,
    waiting: bool,
    pending: Option<Arc<CompareRequest>>,
    cancellation: Option<CompareId>,
    tree_pending: [Option<(TreeViewStamp, SelectionRevision)>; 2],
    /// The last result's status line, with each pane's generation and selection
    /// revision as the result left them: it is shown while both panes still show
    /// the selections it made, and never as a window.
    reported: Option<(String, [(u64, SelectionRevision); 2])>,
    /// The running comparison's latest progress as one status line; shown only
    /// while that comparison waits or runs (Compare contents, Include subfolders).
    pub(crate) progress: String,
}

impl ComparisonState {
    /// A comparison waits for, or runs against, the exact displayed rows of both
    /// panes. Automatic re-reads wait for it instead of cancelling it.
    pub(crate) fn in_progress(&self) -> bool {
        self.waiting || self.pending.is_some()
    }

    /// One that reads contents or walks subfolders, which can take minutes.
    fn long(&self) -> bool {
        self.in_progress() && (self.options.by_content || self.options.recursive)
    }

    /// Shutdown hands captured listings and selections to reclamation.
    pub(super) fn retire_captures(&mut self, retired: &mut Vec<DeferredDrop>) {
        retired.push(Box::new(self.pending.take()));
        retired.push(Box::new(mem::take(&mut self.seeds)));
    }
}

impl Presenter {
    /// While a long comparison runs, both panels keep the rows it compares and
    /// re-read their folders only once it ends (`background_refresh_blocked`),
    /// so a change made in them now would show minutes late. `action`, a change
    /// of a panel's folder the user expects to see at once, is refused with that
    /// explanation instead. Returns whether it was refused.
    pub(crate) fn comparison_refuses(&mut self, action: &str) -> bool {
        if !self.comparison.long() {
            return false;
        }
        self.desktop.report(
            "Directory comparison running",
            format!(
                "{action} is not available while the directory comparison reads both panels' folders: they show changes only once it ends. Wait for it to finish, or open Compare directories and cancel it."
            ),
        );
        true
    }

    /// An outcome the dialog cannot show once it closed (it closes on Compare) is
    /// reported as a notice; an open dialog shows it in its own error line.
    fn report_comparison(&mut self, title: &str, message: String) {
        if self.comparison.open {
            self.comparison.error = message;
        } else {
            self.desktop.report(title, message);
        }
    }

    /// The running comparison's progress, or the last comparison's result while
    /// both panes still show the selections it made.
    pub(crate) fn comparison_status(&self) -> Option<&str> {
        if self.comparison.in_progress() && !self.comparison.progress.is_empty() {
            return Some(&self.comparison.progress);
        }
        let (line, panes) = self.comparison.reported.as_ref()?;
        self.panes
            .iter()
            .zip(panes)
            .all(|(pane, (generation, revision))| {
                pane.generation == *generation && tree::selection_revision(pane) == *revision
            })
            .then_some(line.as_str())
    }

    pub(crate) fn open_comparison(&mut self) {
        // A new comparison would select differences in a folder hidden under
        // search results; a running one can still be reopened and cancelled.
        if !self.comparison.in_progress()
            && [PaneId::Left, PaneId::Right]
                .into_iter()
                .any(|pane| self.feed(pane).is_some())
        {
            self.feed_refuse("Compare directories");
            return;
        }
        if self.panes.iter().any(|p| !p.native_capabilities()) {
            self.desktop.report(
                "Archive comparison unavailable",
                "Directory comparison currently requires two native filesystem panels.".into(),
            );
            return;
        }
        self.comparison.open = true;
        self.comparison.error.clear();
    }

    pub(crate) fn submit_comparison(
        &mut self,
        options: CompareOptions,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.comparison.busy || self.panes.iter().any(|pane| !pane.native_capabilities()) {
            return;
        }
        if options.time_tolerance_seconds > 86_400 {
            self.comparison.error = "Time tolerance must be between 0 and 86400 seconds.".into();
            return;
        }
        if [PaneId::Left, PaneId::Right]
            .iter()
            .any(|pane| self.visible_directory(*pane).is_none())
        {
            self.comparison.error = "Wait until both directories have finished loading.".into();
            return;
        }
        // Reading contents or walking subfolders can take a while: the status line
        // shows that it runs from the start, then its progress.
        self.comparison.progress = if options.by_content || options.recursive {
            "Comparing directories…".into()
        } else {
            String::new()
        };
        self.comparison.options = options;
        self.comparison.error.clear();
        self.comparison.summary.clear();
        self.retired.push(Box::new(self.comparison.reported.take()));
        for pane in [PaneId::Left, PaneId::Right] {
            self.change_search_query(pane, "", send);
        }
        self.comparison.busy = true;
        self.comparison.waiting = true;
        self.flush_comparison(send);
        // The panels show the result by selecting the differences, so the dialog gets out of
        // the way at once; closing it here, unlike cancel, lets the comparison run on.
        self.comparison.open = false;
    }

    pub(crate) fn cancel_comparison(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(request) = self.comparison.pending.take() {
            self.comparison.cancellation = Some(request.id);
            self.retired.push(Box::new(request));
        }
        self.comparison.waiting = false;
        self.comparison.busy = false;
        self.comparison.open = false;
        self.flush_comparison(send);
    }

    fn comparison_current(&self, request: &CompareRequest) -> bool {
        request.sides.iter().enumerate().all(|(slot, side)| {
            let pane = &self.panes[slot];
            !pane.loading
                && !pane.search.pending
                && !pane.search_view_waiting()
                && tree::selection_ready(pane)
                && tree::selection_revision(pane) == side.selection_revision
                && match (&side.tree, pane.tree_view()) {
                    (Some(captured), Some(current)) => Arc::ptr_eq(captured, current),
                    (None, None) => true,
                    _ => false,
                }
                && pane.search.view_revision == side.view_revision
                && pane.successful_generation == Some(side.source.navigation)
                && pane.path == side.source.path
                && Arc::ptr_eq(&pane.entries, &side.source.entries)
                && Arc::ptr_eq(pane.display_index(), &side.view)
        })
    }

    pub fn flush_comparison(&mut self, send: &dyn Fn(Command) -> bool) {
        self.observe_tree_comparison();
        if self
            .comparison
            .pending
            .as_ref()
            .is_some_and(|request| !self.comparison_current(request))
        {
            let request = self.comparison.pending.take().unwrap();
            self.comparison.cancellation = Some(request.id);
            self.retired.push(Box::new(request));
            self.comparison.busy = false;
            self.report_comparison(
                "Directory comparison cancelled",
                "Comparison cancelled because a panel or selection changed.".into(),
            );
        }
        if let Some(id) = self.comparison.cancellation
            && send(Command::CancelComparison(id))
        {
            self.comparison.cancellation = None;
        }
        if !self.comparison.waiting {
            return;
        }
        let mut sides = Vec::with_capacity(2);
        for pane in [PaneId::Left, PaneId::Right] {
            let Some(source) = self.visible_directory(pane) else {
                self.comparison.waiting = false;
                self.comparison.busy = false;
                self.report_comparison(
                    "Directory comparison cancelled",
                    "Comparison cancelled because a directory changed.".into(),
                );
                return;
            };
            let p = &self.panes[pane.index()];
            if p.tree.unavailable {
                // A dropped tree session never becomes ready; the panel is listed
                // again instead, and that re-read must not wait for this request.
                self.comparison.waiting = false;
                self.comparison.busy = false;
                self.report_comparison(
                    "Directory comparison cancelled",
                    "Comparison cancelled because a panel or selection changed.".into(),
                );
                return;
            }
            if p.search.pending || p.search_view_waiting() || !tree::selection_ready(p) {
                return;
            }
            sides.push(CompareSide {
                source,
                tree: p.tree_view().cloned(),
                view: p.display_index().clone(),
                view_revision: p.search.view_revision,
                selection_revision: tree::selection_revision(p),
            });
        }
        let Some(next) = self.comparison.next.checked_add(1) else {
            self.comparison.waiting = false;
            self.comparison.busy = false;
            self.report_comparison(
                "Directory comparison failed",
                "Comparison identities are exhausted.".into(),
            );
            return;
        };
        let request = CompareRequest {
            id: CompareId(next),
            sides: sides.try_into().unwrap(),
            options: self.comparison.options.clone(),
        };
        if send(Command::CompareDirectories(request.clone())) {
            self.comparison.next = next;
            self.comparison.pending = Some(Arc::new(request));
            self.comparison.waiting = false;
        }
    }

    pub(super) fn apply_comparison_event(&mut self, event: Event) -> Option<Event> {
        let (id, result) = match event {
            Event::DirectoriesCompared { id, result } => (id, result),
            Event::ComparisonProgress { id, progress } => {
                if self
                    .comparison
                    .pending
                    .as_ref()
                    .is_some_and(|request| request.id == id)
                {
                    self.comparison.progress = progress_line(&progress, &self.number_locale);
                }
                return None;
            }
            event => return Some(event),
        };
        let Some(request) = self
            .comparison
            .pending
            .as_ref()
            .filter(|request| request.id == id)
        else {
            self.retired.push(Box::new(result));
            return None;
        };
        let valid = self.comparison_current(request);
        self.retired.push(Box::new(self.comparison.pending.take()));
        self.comparison.busy = false;
        if !valid {
            self.report_comparison(
                "Directory comparison cancelled",
                "Comparison discarded because a panel or selection changed.".into(),
            );
            self.retired.push(Box::new(result));
            return None;
        }
        match result {
            Err(error) => {
                let message = crate::error_presentation::describe(&error);
                self.report_comparison("Directory comparison failed", message);
            }
            Ok(result) => {
                if !self.comparison_current(&result.request)
                    || self
                        .panes
                        .iter()
                        .any(|pane| tree::selection_revision(pane).0 == u64::MAX)
                    || !tree::result_matches(&result)
                {
                    self.report_comparison(
                        "Directory comparison cancelled",
                        "Comparison no longer matches the displayed panels.".into(),
                    );
                    self.retired.push(Box::new(result));
                    return None;
                }
                for (slot, (pane, selected)) in
                    self.panes.iter_mut().zip(&result.selections).enumerate()
                {
                    if let Some(selection) = &result.tree_replacements[slot] {
                        let revision = SelectionRevision(pane.tree.revision.0 + 1);
                        let action = TreeSelectionAction::Replace(selection.clone());
                        pane.tree.revision = revision;
                        pane.tree.preview.push((revision, action.clone()));
                        pane.tree.replay.push_back((revision, action));
                        self.comparison.tree_pending[slot] = Some((selection.stamp(), revision));
                        continue;
                    }
                    let seed = Arc::new(AuthoritativeSelection {
                        id: result.request.id.0,
                        origin: SelectionOrigin {
                            navigation: pane.selection_context,
                            location: pane.path.clone(),
                        },
                        snapshot: selected.clone(),
                    });
                    self.retired
                        .push(Box::new(self.comparison.seeds[slot].replace(seed)));
                    pane.selection_revision.0 += 1;
                    pane.selection_applied = pane.selection_revision;
                    pane.selection_resolved = pane.selection_revision;
                    pane.selection_dirty = true;
                    pane.selection_preview.clear();
                    self.retired
                        .push(Box::new(mem::replace(&mut pane.selected, selected.clone())));
                }
                self.comparison.summary = summary(&result, &self.number_locale);
                // The panels show the differences selected; one line also says how
                // many, with no window (the dialog has closed).
                let line = status_line(&result, &self.number_locale);
                let shown = [0, 1].map(|slot| {
                    let pane = &self.panes[slot];
                    (pane.generation, tree::selection_revision(pane))
                });
                self.retired
                    .push(Box::new(self.comparison.reported.replace((line, shown))));
                self.comparison.busy = self.comparison.tree_pending.iter().any(Option::is_some);
                self.retired.push(Box::new(result));
            }
        }
        None
    }
}

/// The dialog's summary of the last result, shown when it is opened again.
fn summary(result: &CompareResult, locale: &NumberLocale) -> String {
    let options = &result.request.options;
    let count = |value: u64| locale.format_u64(value);
    let differences = result
        .selections
        .iter()
        .any(|selection| !selection.is_empty())
        || result.unknown > 0
        || result.ambiguous > 0;
    let mut text = if differences {
        format!(
            "Found {} differences in the first directory and {} in the second directory. Unknown metadata: {}. Ambiguous name groups: {}.",
            result.selections[0].len(),
            result.selections[1].len(),
            result.unknown,
            result.ambiguous
        )
    } else if options.by_content {
        "Displayed metadata and the compared contents match for the chosen criteria.".into()
    } else {
        "Displayed metadata matches for the chosen criteria.".into()
    };
    if options.recursive {
        text.push_str(&format!(
            " Folders with differences: {}.",
            count(result.differing_folders as u64)
        ));
    }
    if options.by_content {
        text.push_str(&format!(
            " Files read byte by byte: {} pairs ({}); different contents: {}.",
            count(result.content_files),
            crate::format_size(result.content_bytes, locale),
            count(result.content_differences as u64)
        ));
    }
    if result.skipped > 0 {
        text.push_str(&format!(
            " Links and special files matched by name only: {}.",
            count(result.skipped)
        ));
    }
    if result.limited {
        text.push_str(
            " A size limit ended the folder walk early; the folders it did not finish are selected.",
        );
    }
    if result.errors > 0 {
        text.push_str(&format!(
            " Not readable: {} (their folders or files are selected).",
            count(result.errors)
        ));
        for error in &result.diagnostics {
            text.push_str("\n\n");
            text.push_str(&crate::error_presentation::describe(error));
        }
    }
    text
}

/// One status line for a running comparison.
fn progress_line(progress: &CompareProgress, locale: &NumberLocale) -> String {
    let count = |value: u64| locale.format_u64(value);
    match progress.phase {
        ComparePhase::Scanning => format!(
            "Comparing subfolders: {} entries in {} folders…",
            count(progress.entries),
            count(progress.folders)
        ),
        ComparePhase::Contents => format!(
            "Comparing contents: {} of {} files · {} of {}…",
            count(progress.content_files),
            count(progress.content_files_total),
            crate::format_size(progress.content_bytes, locale),
            crate::format_size(progress.content_bytes_total, locale)
        ),
    }
}

/// "Compared: …" for the status line, with the counts each panel now selects.
fn status_line(result: &CompareResult, locale: &NumberLocale) -> String {
    let [left, right] = result
        .selections
        .each_ref()
        .map(|selection| selection.len());
    let count = |value: usize| locale.format_u64(value as u64);
    let mut line = if left == 0 && right == 0 && result.unknown == 0 && result.ambiguous == 0 {
        "Compared: no differences for the chosen criteria".to_owned()
    } else {
        format!(
            "Compared: {} selected in the left panel, {} in the right",
            count(left),
            count(right)
        )
    };
    if result.differing_folders > 0 {
        line.push_str(&format!(
            " · {} {} with differences",
            count(result.differing_folders),
            if result.differing_folders == 1 {
                "folder"
            } else {
                "folders"
            }
        ));
    }
    if result.content_differences > 0 {
        line.push_str(&format!(
            " · {} with different contents",
            count(result.content_differences)
        ));
    }
    if result.request.options.by_content {
        line.push_str(&format!(
            " · {} file {} read",
            locale.format_u64(result.content_files),
            if result.content_files == 1 {
                "pair"
            } else {
                "pairs"
            }
        ));
    }
    if result.unknown > 0 {
        line.push_str(&format!(
            " · {} with unknown metadata",
            count(result.unknown)
        ));
    }
    if result.ambiguous > 0 {
        line.push_str(&format!(
            " · {} ambiguous name groups",
            count(result.ambiguous)
        ));
    }
    if result.errors > 0 {
        line.push_str(&format!(
            " · {} not readable",
            locale.format_u64(result.errors)
        ));
    }
    if result.limited {
        line.push_str(" · stopped at a size limit");
    }
    line
}
