//! Atomic publication of two immutable, revision-checked selection snapshots, the
//! running comparison's progress (in the Compare directories dialog) and its result
//! (a line below the panel that was active, and per-row marks).
use super::*;
#[cfg(test)]
#[path = "comparison_badge_tests.rs"]
mod badge_tests;
mod describe;
#[cfg(test)]
#[path = "comparison_tests.rs"]
mod tests;
mod tree;

/// What the dialog's ignore fields hold, kept while their boxes are off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct IgnoreChoices {
    pub(crate) files: bool,
    pub(crate) files_masks: String,
    pub(crate) folders: bool,
    pub(crate) folders_masks: String,
}

/// The last result as the panels show it: its line, the panel it is shown below,
/// and each panel's generation and selection revision as the result left them.
pub(crate) struct Reported {
    pub(crate) line: String,
    pub(crate) details: String,
    pub(crate) pane: PaneId,
    shown: [(u64, SelectionRevision); 2],
}

/// Per-row outcomes of the last comparison, while both panels still
/// show the listings it compared (`generation` of each), with the options that
/// tell what "matches" meant and the panel whose line describes them.
pub(crate) struct ShownMarks {
    marks: Arc<CompareMarks>,
    generations: [u64; 2],
    options: CompareOptions,
    pane: PaneId,
    /// The details of the line that names the marks once the result line is gone.
    details: String,
}

#[derive(Default)]
pub(crate) struct ComparisonState {
    pub(crate) open: bool,
    pub(crate) busy: bool,
    pub(crate) error: String,
    pub(crate) summary: String,
    pub(crate) options: CompareOptions,
    pub(crate) ignore: IgnoreChoices,
    pub(super) seeds: [Option<Arc<AuthoritativeSelection>>; 2],
    next: u64,
    waiting: bool,
    pending: Option<Arc<CompareRequest>>,
    cancellation: Option<CompareId>,
    tree_pending: [Option<(TreeViewStamp, SelectionRevision)>; 2],
    /// The panel that was active when Compare was pressed: the result shows below it.
    origin: Option<PaneId>,
    pub(crate) reported: Option<Reported>,
    pub(crate) marks: Option<ShownMarks>,
    /// The running comparison's latest progress sample.
    pub(crate) progress: Option<CompareProgress>,
}

impl ComparisonState {
    /// A comparison waits for, or runs against, the exact displayed rows of both
    /// panes. Automatic re-reads wait for it instead of cancelling it.
    pub(crate) fn in_progress(&self) -> bool {
        self.waiting || self.pending.is_some()
    }

    /// One that reads contents or walks subfolders, which can take minutes: the
    /// dialog stays open with its progress and Cancel.
    pub(crate) fn running(&self) -> bool {
        self.in_progress() && (self.options.by_content || self.options.recursive)
    }

    /// Shutdown hands captured listings and selections to reclamation.
    pub(super) fn retire_captures(&mut self, retired: &mut Vec<DeferredDrop>) {
        retired.push(Box::new(self.pending.take()));
        retired.push(Box::new(mem::take(&mut self.seeds)));
        retired.push(Box::new(self.marks.take()));
    }
}

impl Presenter {
    /// While a long comparison runs, both panels keep the rows it compares and
    /// re-read their folders only once it ends (`background_refresh_blocked`),
    /// so a change made in them now would show minutes late. `action`, a change
    /// of a panel's folder the user expects to see at once, is refused with that
    /// explanation instead. Returns whether it was refused.
    pub(crate) fn comparison_refuses(&mut self, action: &str) -> bool {
        if !self.comparison.running() {
            return false;
        }
        self.desktop.report(
            "Directory comparison running",
            format!(
                "{action} is not available while the directory comparison reads both panels' folders: they show changes only once it ends. Wait for it to finish, or press Cancel (Escape) in the Compare directories dialog."
            ),
        );
        true
    }

    /// An outcome the dialog cannot show once it closed is reported as a notice;
    /// an open dialog shows it in its own error line.
    fn report_comparison(&mut self, title: &str, message: String) {
        if self.comparison.open {
            self.comparison.error = message;
        } else {
            self.desktop.report(title, message);
        }
    }

    /// The last comparison's result line below `pane`, with its details, while both
    /// panes still show the selections it made. Once a selection changes, the line
    /// says that the panels still show the comparison's marks, as long as they do.
    pub(crate) fn comparison_note(&self, pane: PaneId) -> Option<(&str, &str)> {
        if let Some(reported) = self.comparison.reported.as_ref()
            && reported.pane == pane
            && self
                .panes
                .iter()
                .zip(&reported.shown)
                .all(|(pane, (generation, revision))| {
                    pane.generation == *generation && tree::selection_revision(pane) == *revision
                })
        {
            return Some((reported.line.as_str(), reported.details.as_str()));
        }
        self.comparison
            .marks
            .as_ref()
            .filter(|shown| shown.pane == pane)
            .map(|shown| {
                (
                    "Comparison marks: = matches, ≠ differs, + only here, ? not verified · × clears them",
                    shown.details.as_str(),
                )
            })
    }

    /// Whether Clear comparison has something to clear: a result line or marks.
    pub(crate) fn comparison_shown(&self) -> bool {
        self.comparison.marks.is_some()
            || [PaneId::Left, PaneId::Right]
                .into_iter()
                .any(|pane| self.comparison_note(pane).is_some())
    }

    /// Clear comparison: the marks leave both panels and the result line its panel.
    /// The selection the comparison made stays, as in Salamander. False when there
    /// was nothing to clear.
    pub(crate) fn clear_comparison(&mut self) -> bool {
        let shown = self.comparison_shown();
        self.retired.push(Box::new(self.comparison.reported.take()));
        self.retired.push(Box::new(self.comparison.marks.take()));
        shown
    }

    /// How to clear the comparison, for the result line's details.
    fn clear_hint(&self) -> String {
        let keys = self.shortcut_hint("panel.compare-clear");
        let keys = if keys.is_empty() {
            String::new()
        } else {
            format!(", {keys}")
        };
        format!(
            "The × at the right end of this line{keys} closes it and clears the marks in both panels; the selection stays. Listing a panel again or a new comparison also clears them."
        )
    }

    /// How the last comparison found `entry` in `pane`: 1 matches,
    /// 2 differs, 3 exists in this panel only, 4 not verified; 0 when there is
    /// nothing to show (another listing, or not compared).
    #[cfg(test)]
    pub(crate) fn comparison_mark(&self, pane: PaneId, entry: &Entry) -> i32 {
        self.comparison_badge(pane, entry, &entry.metadata).0
    }

    /// Marks that no longer describe both panels' listings are dropped.
    fn observe_comparison_marks(&mut self) {
        if self.comparison.marks.as_ref().is_some_and(|shown| {
            self.panes
                .iter()
                .zip(shown.generations)
                .any(|(pane, generation)| pane.generation != generation)
        }) {
            self.retired.push(Box::new(self.comparison.marks.take()));
        }
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
        if !self.comparison.running() {
            self.comparison.summary.clear();
        }
    }

    pub(crate) fn submit_comparison(
        &mut self,
        options: CompareOptions,
        ignore: IgnoreChoices,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.comparison.busy || self.panes.iter().any(|pane| !pane.native_capabilities()) {
            return;
        }
        if let Err(error) = options.validate() {
            self.comparison.error = error.into();
            return;
        }
        if [PaneId::Left, PaneId::Right]
            .iter()
            .any(|pane| self.visible_directory(*pane).is_none())
        {
            self.comparison.error = "Wait until both directories have finished loading.".into();
            return;
        }
        if self.panes[0].path == self.panes[1].path {
            self.comparison.error =
                "Both panels show the same folder. Open another folder in one of them.".into();
            return;
        }
        self.comparison.options = options;
        self.comparison.ignore = ignore;
        self.comparison.origin = Some(self.active);
        self.comparison.progress = None;
        self.comparison.error.clear();
        self.comparison.summary.clear();
        self.retired.push(Box::new(self.comparison.reported.take()));
        self.retired.push(Box::new(self.comparison.marks.take()));
        for pane in [PaneId::Left, PaneId::Right] {
            self.change_search_query(pane, "", send);
        }
        self.comparison.busy = true;
        self.comparison.waiting = true;
        self.flush_comparison(send);
        // A quick comparison's result is the panels' selection: the dialog gets out of
        // the way at once. A long one keeps the dialog open with its progress and
        // Cancel until it ends.
        if !self.comparison.running() {
            self.comparison.open = false;
        }
    }

    /// Cancel (the button, Escape) or Close: a running comparison stops and changes
    /// nothing; the dialog closes.
    pub(crate) fn cancel_comparison(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(request) = self.comparison.pending.take() {
            self.comparison.cancellation = Some(request.id);
            self.retired.push(Box::new(request));
        }
        self.comparison.waiting = false;
        self.comparison.busy = self.comparison.tree_pending.iter().any(Option::is_some);
        self.comparison.progress = None;
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

    /// The comparison stopped without a result: the dialog says why when it is open.
    fn comparison_stopped(&mut self, title: &str, message: &str) {
        self.comparison.busy = self.comparison.tree_pending.iter().any(Option::is_some);
        self.comparison.progress = None;
        self.report_comparison(title, message.into());
    }

    pub fn flush_comparison(&mut self, send: &dyn Fn(Command) -> bool) {
        self.observe_tree_comparison();
        self.observe_comparison_marks();
        if self
            .comparison
            .pending
            .as_ref()
            .is_some_and(|request| !self.comparison_current(request))
        {
            let request = self.comparison.pending.take().unwrap();
            self.comparison.cancellation = Some(request.id);
            self.retired.push(Box::new(request));
            self.comparison_stopped(
                "Directory comparison cancelled",
                "Comparison cancelled because a panel or selection changed.",
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
                self.comparison_stopped(
                    "Directory comparison cancelled",
                    "Comparison cancelled because a directory changed.",
                );
                return;
            };
            let p = &self.panes[pane.index()];
            if p.tree.unavailable {
                // A dropped tree session never becomes ready; the panel is listed
                // again instead, and that re-read must not wait for this request.
                self.comparison.waiting = false;
                self.comparison_stopped(
                    "Directory comparison cancelled",
                    "Comparison cancelled because a panel or selection changed.",
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
            self.comparison_stopped(
                "Directory comparison failed",
                "Comparison identities are exhausted.",
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
                    self.retired
                        .push(Box::new(self.comparison.progress.replace(progress)));
                } else {
                    self.retired.push(Box::new(progress));
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
        let long = self.comparison.running();
        self.retired.push(Box::new(self.comparison.pending.take()));
        self.retired.push(Box::new(self.comparison.progress.take()));
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
                let summary = summary(&result, &self.number_locale);
                // The panels show the differences selected; a line below the panel that
                // was active says how many. Problems keep the dialog open with them.
                let origin = self.comparison.origin.unwrap_or(self.active);
                let mut line = status_line(&result, origin, &self.number_locale);
                let mut details = summary.clone();
                let clear = self.clear_hint();
                if result.marks.is_some() {
                    // Marks stay until cleared: the line says how.
                    let key = self.shortcut_key("panel.compare-clear");
                    if key.is_empty() {
                        line.push_str(" · × clears the marks");
                    } else {
                        line.push_str(&format!(" · {key} or × clears the marks"));
                    }
                    details.push_str("\n\n");
                    details.push_str(&clear);
                }
                let shown = [0, 1].map(|slot| {
                    let pane = &self.panes[slot];
                    (pane.generation, tree::selection_revision(pane))
                });
                self.retired
                    .push(Box::new(self.comparison.reported.replace(Reported {
                        line,
                        details,
                        pane: origin,
                        shown,
                    })));
                if let Some(marks) = &result.marks {
                    let generations = [0, 1].map(|slot| self.panes[slot].generation);
                    self.retired
                        .push(Box::new(self.comparison.marks.replace(ShownMarks {
                            marks: marks.clone(),
                            generations,
                            options: result.request.options.clone(),
                            pane: origin,
                            details: format!("Point at a mark to see what differs.\n\n{clear}"),
                        })));
                }
                if long && (result.errors > 0 || result.limited) {
                    self.comparison.summary = summary;
                } else {
                    self.comparison.open = false;
                }
                self.comparison.busy = self.comparison.tree_pending.iter().any(Option::is_some);
                self.retired.push(Box::new(result));
            }
        }
        None
    }
}

/// The result's details: the line's tooltip, and the dialog's summary when the
/// comparison could not compare everything.
fn summary(result: &CompareResult, locale: &NumberLocale) -> String {
    let options = &result.request.options;
    let count = |value: u64| locale.format_u64(value);
    let mut text = if result.identical() {
        identical_text(options).to_owned()
    } else {
        format!(
            "Selected: {} in the left panel, {} in the right panel.",
            count(result.selections[0].len() as u64),
            count(result.selections[1].len() as u64),
        )
    };
    if options.recursive {
        text.push_str(&format!(
            " Folders compared: {} ({} folder pairs and {} file pairs inside); with differences: {}.",
            count(result.compared_folders as u64),
            count(result.walked_folders),
            count(result.walked_files),
            count(result.differing_folders as u64),
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
    if result.dst_shifts > 0 {
        text.push_str(&format!(
            " {}",
            dst_text(result.dst_shifts, options.ignore_dst_shifts, locale)
        ));
    }
    if result.unknown > 0 || result.ambiguous > 0 {
        text.push_str(&format!(
            " Unknown metadata or not verified: {}. Names that differ only in case: {}.",
            count(result.unknown as u64),
            count(result.ambiguous as u64)
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
            " A size limit ended part of the folder walk; the folders it did not finish are selected.",
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

/// Salamander's verdict when nothing differs.
fn identical_text(options: &CompareOptions) -> &'static str {
    if options.by_content {
        "The files in the two folders are identical."
    } else {
        "The files in the two folders appear to be identical."
    }
}

fn dst_text(shifts: u64, ignored: bool, locale: &NumberLocale) -> String {
    let pairs = if shifts == 1 { "pair" } else { "pairs" };
    if ignored {
        format!(
            "{} file {pairs} one or two hours apart (daylight saving) counted as equal.",
            locale.format_u64(shifts)
        )
    } else {
        format!(
            "{} file {pairs} differ by exactly one or two hours, probably daylight saving; Compare directories can ignore that.",
            locale.format_u64(shifts)
        )
    }
}

/// The running comparison's progress for the dialog: the counts, the bytes read,
/// and the fraction done (below zero when it cannot be told).
pub(crate) fn progress_texts(
    progress: Option<&CompareProgress>,
    locale: &NumberLocale,
) -> (String, String, f32) {
    let Some(progress) = progress else {
        return ("Starting…".into(), String::new(), -1.0);
    };
    let count = |value: u64| locale.format_u64(value);
    let mut counts = if progress.top_total > 0 {
        format!(
            "Folder {} of {} · {} folders and {} files compared",
            count((progress.top_done + 1).min(progress.top_total)),
            count(progress.top_total),
            count(progress.folders),
            count(progress.files),
        )
    } else {
        "Comparing the displayed files".to_owned()
    };
    counts.push_str(&format!(
        " · {} {} found",
        count(progress.differences),
        if progress.differences == 1 {
            "difference"
        } else {
            "differences"
        }
    ));
    let detail = if progress.content_files_total > 0 {
        format!(
            "Contents: {} of {} file pairs read · {} of {}",
            count(progress.content_files),
            count(progress.content_files_total),
            crate::format_size(progress.content_bytes, locale),
            crate::format_size(progress.content_bytes_total, locale)
        )
    } else {
        String::new()
    };
    (counts, detail, progress.fraction().unwrap_or(-1.0))
}

/// "Compared: …" below the panel that was active, with the counts each panel now
/// selects (that panel's first).
fn status_line(result: &CompareResult, pane: PaneId, locale: &NumberLocale) -> String {
    let options = &result.request.options;
    if result.identical() {
        let mut line = format!("Compared: {}", identical_text(options));
        if result.dst_shifts > 0 {
            line.push_str(&format!(
                " · {} one- or two-hour shifts ignored",
                locale.format_u64(result.dst_shifts)
            ));
        }
        return line;
    }
    let count = |value: usize| locale.format_u64(value as u64);
    let [left, right] = result
        .selections
        .each_ref()
        .map(|selection| selection.len());
    let (here, there, other) = match pane {
        PaneId::Left => (left, right, "right"),
        PaneId::Right => (right, left, "left"),
    };
    let mut line = format!(
        "Compared: {} selected here, {} in the {other} panel",
        count(here),
        count(there)
    );
    if options.recursive {
        line.push_str(&format!(
            " · {} of {} folders differ",
            count(result.differing_folders),
            count(result.compared_folders)
        ));
    }
    if result.content_differences > 0 {
        line.push_str(&format!(
            " · {} with different contents",
            count(result.content_differences)
        ));
    }
    if result.dst_shifts > 0 {
        line.push_str(&format!(
            " · {} one- or two-hour time {}",
            locale.format_u64(result.dst_shifts),
            if options.ignore_dst_shifts {
                "shifts ignored"
            } else {
                "differences (daylight saving?)"
            }
        ));
    }
    if result.unknown > 0 {
        line.push_str(&format!(" · {} not verified", count(result.unknown)));
    }
    if result.ambiguous > 0 {
        line.push_str(&format!(
            " · {} names differ only in case",
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
