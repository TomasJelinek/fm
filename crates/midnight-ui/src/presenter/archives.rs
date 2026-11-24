//! Additive virtual pane state. Native paths remain only the captured return location.
use super::*;

#[derive(Clone)]
pub(super) struct ArchivePane {
    pub(super) sizes: super::archive_sizes::State,
    pub(super) location: ArchiveLocation,
    pub(super) origin: PathBuf,
    pub(super) wanted_cursor: Option<VirtualEntryId>,
    pub(super) selected: Arc<HashSet<EntryId>>,
    /// Where the selected members are shown, for jumps between them; built on the first
    /// jump after the selection or the shown rows changed (`selected_navigation`).
    pub(super) selected_positions: super::selected_navigation::SelectedPositions,
    pub(super) totals: ArchiveSelectionTotals,
    pub(super) selection_pending: bool,
    pub(super) selection_revision: SelectionRevision,
    /// The mounted folder of the listing in flight, kept for its recovery.
    requested: Option<ArchiveLocation>,
    recovery: Recovery,
    /// An archive inside an archive that is being opened, for its status-bar progress.
    opening: Option<Opening>,
}

/// A failed mounted listing may mean that the archive's session stopped (a stalled
/// provider worker is stopped by its watchdog), and every later request through that
/// session would fail too. The pane then never lists through it again: it re-opens
/// the archive once, and leaves the archive if that fails as well. An archive inside
/// an archive is not re-opened: the pane returns to the folder that contains it.
#[derive(Clone, Default)]
enum Recovery {
    #[default]
    None,
    /// List this folder again through a fresh session.
    Reopen(ArchiveLocation),
    /// That listing is in flight.
    Reopening,
    /// The re-open failed: return to the native folder that contains the archive.
    Leave,
    /// The failed session read an archive inside another archive: list the enclosing
    /// folder, with the cursor on that archive.
    Parent(ArchiveParent),
}

/// An archive inside an archive whose member is being staged by `navigation`.
#[derive(Clone)]
struct Opening {
    navigation: NavigationId,
    name: String,
    size: Option<u64>,
    /// The progress this pane put in the status bar, removed when the open ends.
    status: Option<String>,
}

impl ArchivePane {
    pub(super) fn clear_tree_selection(&mut self, retired: &mut Vec<DeferredDrop>) {
        retired.push(Box::new(mem::take(&mut self.selected)));
        self.totals = ArchiveSelectionTotals::default();
        self.selection_pending = false;
    }
    pub(super) fn unavailable(address: ArchiveAddress, origin: PathBuf) -> Self {
        Self {
            sizes: Default::default(),
            location: ArchiveLocation::unavailable(address),
            origin,
            wanted_cursor: None,
            selected: Arc::default(),
            selected_positions: Default::default(),
            totals: ArchiveSelectionTotals::default(),
            selection_pending: false,
            selection_revision: SelectionRevision(0),
            requested: None,
            recovery: Recovery::None,
            opening: None,
        }
    }
    /// Keep the addresses of the shown and remembered folders without their session:
    /// listing one of them then opens the archive again instead of using that session.
    /// Folders of archives inside archives keep their own sessions, and going up from
    /// them reaches their enclosing folders.
    fn forget_session(&mut self, history: &mut [HistoryEntry], retired: &mut Vec<DeferredDrop>) {
        for location in std::iter::once(&mut self.location).chain(history::archives_mut(history)) {
            if location.mount.is_some() && location.parent().is_none() {
                let unavailable = ArchiveLocation::unavailable(location.address.clone());
                retired.push(Box::new(mem::replace(location, unavailable)));
            }
        }
    }
    /// The folders that path parts navigate to, outermost first: the folders of each
    /// enclosing archive down to the one that holds the next archive, then this
    /// archive's own folders.
    pub(super) fn path_locations(&self) -> Vec<ArchiveLocation> {
        let mut levels: Vec<ArchiveLocation> = self
            .location
            .enclosing()
            .into_iter()
            .map(|parent| parent.location)
            .collect();
        levels.reverse();
        levels.push(self.location.clone());
        let mut parts = Vec::new();
        for level in levels {
            for (index, (node, _)) in level.trail.iter().enumerate() {
                let mut part = level.clone();
                part.directory = *node;
                part.trail.truncate(index + 1);
                part.address.truncate_steps(index);
                parts.push(part);
            }
        }
        parts
    }
    /// The location saved for this pane in tabs and Hot Paths, with every archive inside
    /// an archive that leads here (`ArchiveLocation::saved_address`).
    pub(super) fn saved_address(&self) -> ArchiveAddress {
        self.location.saved_address()
    }
    /// A remembered root of the archive opened from `member`, whose session can be
    /// entered again instead of staging the member once more.
    fn remembered_inner_root(
        history: &[HistoryEntry],
        member: VirtualEntryId,
    ) -> Option<ArchiveLocation> {
        let mut root = history::archives(history)
            .find(|location| {
                location
                    .parent()
                    .is_some_and(|parent| parent.member == member)
            })?
            .clone();
        root.directory = 1;
        root.trail.truncate(1);
        root.address.components.clear();
        Some(root)
    }
    /// Every location through which this tab holds a session: the shown one, its
    /// history, the listing in flight and a pending recovery.
    fn held<'a>(
        &'a self,
        history: &'a [HistoryEntry],
    ) -> impl Iterator<Item = &'a ArchiveLocation> {
        let recovery = match &self.recovery {
            Recovery::Parent(parent) => Some(&parent.location),
            _ => None,
        };
        std::iter::once(&self.location)
            .chain(history::archives(history))
            .chain(&self.requested)
            .chain(recovery)
            .filter(|location| location.mount.is_some())
    }
    /// Whether this tab's listing in flight re-opens its archive after a failed listing
    /// (`Recovery::Reopening`).
    pub(super) fn reopening(&self) -> bool {
        matches!(self.recovery, Recovery::Reopening)
    }
    /// The session of the archive opened from a native file that this tab holds.
    pub(super) fn native_session(&self, history: &[HistoryEntry]) -> Option<u64> {
        self.held(history)
            .find_map(|location| location.outermost().mount_id())
    }
    /// Whether this tab holds archive session `session`, also through an archive inside
    /// it.
    pub(super) fn holds_session(&self, history: &[HistoryEntry], session: u64) -> bool {
        self.held(history).any(|location| {
            location.mount_id() == Some(session)
                || location
                    .enclosing()
                    .iter()
                    .any(|parent| parent.location.mount_id() == Some(session))
        })
    }
    /// Let go of every session this tab holds, keeping their saved addresses: listing one
    /// of them again opens its archive, and the archives inside it, again.
    fn let_go(&mut self, history: &mut [HistoryEntry], retired: &mut Vec<DeferredDrop>) {
        if self.location.mount.is_some() {
            let unavailable = ArchiveLocation::unavailable(self.location.saved_address());
            retired.push(Box::new(mem::replace(&mut self.location, unavailable)));
        }
        history::release_sessions(history, retired);
        retired.push(Box::new(self.requested.take()));
        retired.push(Box::new(mem::take(&mut self.recovery)));
    }
    /// The archive sessions of this pane's location: its own and its enclosing ones.
    fn shown_sessions(&self) -> Vec<u64> {
        std::iter::once(self.location.mount_id())
            .chain(
                self.location
                    .enclosing()
                    .into_iter()
                    .map(|parent| parent.location.mount_id()),
            )
            .flatten()
            .collect()
    }
}

/// Where the cursor goes when `target` encloses `current`: on the folder, or on the
/// archive inside an archive, that leads back to `current`.
fn cursor_toward(current: &ArchiveLocation, target: &ArchiveLocation) -> Option<VirtualEntryId> {
    let mount = target.mount_id()?;
    let levels = std::iter::once((current.clone(), None)).chain(
        current
            .enclosing()
            .into_iter()
            .map(|parent| (parent.location, Some(parent.member))),
    );
    for (level, member) in levels {
        if level.mount_id() != Some(mount) {
            continue;
        }
        if level.trail.len() < target.trail.len()
            || !level
                .trail
                .iter()
                .zip(&target.trail)
                .all(|(old, new)| old.0 == new.0)
        {
            return None;
        }
        return match level.trail.get(target.trail.len()) {
            Some((entry, _)) => Some(VirtualEntryId {
                mount,
                entry: *entry,
            }),
            None => member,
        };
    }
    None
}

impl Presenter {
    pub(crate) fn archive_format_available(&self, format: ArchiveFormat) -> bool {
        self.archive_formats.contains(&format)
    }
    pub(crate) fn can_browse_archive(&self, entry: &Entry) -> bool {
        self.browsable_archive_format(entry).is_some()
    }
    pub(crate) fn browsable_archive_format(&self, entry: &Entry) -> Option<ArchiveFormat> {
        self.configured_archive_format(entry)
            .filter(|format| self.archive_formats.contains(format))
    }
    /// Activation follows the user's association while backend discovery settles.
    /// An unavailable provider reports an internal error instead of launching an OS handler.
    /// A readable archive member with an enabled archive name opens as an archive inside
    /// its archive.
    pub(super) fn configured_archive_format(&self, entry: &Entry) -> Option<ArchiveFormat> {
        if entry.kind != EntryKind::File
            || entry
                .virtual_node
                .as_ref()
                .is_some_and(|node| !node.readable)
        {
            return None;
        }
        entry
            .archive_extensions
            .enabled_format(&self.preferences.values)
    }
    /// The enabled, installed archive format of an archive member's name.
    fn member_archive_format(&self, node: &ArchiveNode) -> Option<ArchiveFormat> {
        if node.kind != EntryKind::File {
            return None;
        }
        ArchiveExtensionHints::from_path(Path::new(&node.display_name))
            .enabled_format(&self.preferences.values)
            .filter(|format| self.archive_formats.contains(format))
    }
    pub(super) fn archive_navigate(
        &mut self,
        pane: PaneId,
        action: ArchiveAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.archive_navigate_impl(pane, action, Reload::Navigate, send)
    }

    /// `Reload::Background` requests are retried by their owner, so a full
    /// queue does not report an error on the panel.
    pub(super) fn archive_navigate_impl(
        &mut self,
        pane: PaneId,
        action: ArchiveAction,
        mode: Reload,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        // An archive opened from a native file gets a new session, which takes the place
        // of the one this tab holds or let go of last (`replaceable_session`).
        let opens_session = match &action {
            ArchiveAction::Open { .. } => true,
            ArchiveAction::List(location) => location.mount.is_none(),
            ArchiveAction::OpenMember { .. } => false,
        };
        let replaces = if opens_session {
            self.replaceable_session(pane)
        } else {
            None
        };
        let p = &mut self.panes[pane.index()];
        let Some(navigation) = p.generation.checked_add(1).map(NavigationId) else {
            return false;
        };
        let same = matches!(&action, ArchiveAction::List(location)
            if p.archive.as_ref().is_some_and(|a| a.location.address == location.address));
        let unavailable = match &action {
            ArchiveAction::List(location) if location.mount.is_none() => Some(location.clone()),
            _ => None,
        };
        let requested = match &action {
            ArchiveAction::List(location) if location.mount.is_some() => Some(location.clone()),
            _ => None,
        };
        let resumes = p.archive_resume.as_ref().is_some_and(|resume| {
            resume.continued_by(&action, p.archive.as_ref().map(|a| &a.location))
        });
        let wanted_cursor = match (&p.archive, &action) {
            (Some(archive), ArchiveAction::List(location)) => {
                cursor_toward(&archive.location, location)
            }
            _ => None,
        };
        if !send(Command::ArchiveNavigate {
            pane,
            navigation,
            action,
            sort: p.sort,
            replaces,
        }) {
            if mode != Reload::Background {
                p.set_error("Archive navigation queue is busy", &mut self.retired);
            }
            return false;
        }
        self.retire_network_sign_in(pane);
        self.clear_tree(pane, Some(navigation));
        let p = &mut self.panes[pane.index()];
        p.generation = navigation.0;
        if !same {
            p.selection_context = navigation;
        }
        p.archive_request = true;
        p.network_request = false;
        self.retired.push(Box::new(p.network_pending.take()));
        p.loading = true;
        p.preserve_viewport = false;
        p.set_error("", &mut self.retired);
        // A saved location is opened again level by level where a password stops the
        // backend (`resume_archive`); a navigation elsewhere ends that.
        if !resumes {
            p.archive_history_request = None;
            self.retired.push(Box::new(mem::replace(
                &mut p.archive_resume,
                unavailable
                    .as_ref()
                    .map(|location| history::ArchiveResume::new(location.address.clone())),
            )));
        }
        if let Some(a) = &mut p.archive {
            a.selection_pending = false;
            a.wanted_cursor = wanted_cursor;
            self.retired
                .push(Box::new(mem::replace(&mut a.requested, requested)));
            // A newer request supersedes a pending recovery.
            self.retired.push(Box::new(mem::take(&mut a.recovery)));
            if let Some(location) = unavailable {
                self.retired
                    .push(Box::new(mem::replace(&mut a.location, location)));
            }
            // The tab leaves the session it held: the new one takes its place.
            if opens_session {
                a.let_go(&mut p.history, &mut self.retired);
            }
        }
        if let Some(scan) = p.size_scan.take() {
            send(Command::CancelFolderSizes { pane, scan });
        }
        // A newer request supersedes an archive inside an archive being opened.
        self.end_opening(pane);
        self.search_navigation_accepted(pane, !same, send);
        drag_drop::invalidate(&self.drag_drop.targets);
        true
    }
    /// Remove the status-bar progress of an open that ended or was superseded.
    fn end_opening(&mut self, pane: PaneId) {
        let opening = self.panes[pane.index()]
            .archive
            .as_mut()
            .and_then(|archive| archive.opening.take());
        if opening
            .and_then(|opening| opening.status)
            .is_some_and(|status| status == self.desktop.status)
        {
            self.desktop.status.clear();
        }
    }
    /// Show how much of the member an archive inside an archive has staged.
    fn archive_progress(&mut self, pane: PaneId, navigation: NavigationId, staged: u64) {
        let p = &mut self.panes[pane.index()];
        if p.generation != navigation.0 {
            return;
        }
        let Some(opening) = p
            .archive
            .as_mut()
            .and_then(|archive| archive.opening.as_mut())
            .filter(|opening| opening.navigation == navigation)
        else {
            return;
        };
        let mib = |bytes: u64| self.number_locale.format_u64(bytes.div_ceil(1024 * 1024));
        let status = match opening.size.filter(|size| *size >= staged) {
            Some(size) => format!(
                "Opening {}: {} of {} MiB…",
                opening.name,
                mib(staged),
                mib(size)
            ),
            None => format!("Opening {}: {} MiB…", opening.name, mib(staged)),
        };
        self.desktop.status = status.clone();
        opening.status = Some(status);
    }
    pub(super) fn apply_archive_event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::ArchiveSelection {
                pane,
                navigation,
                revision,
                view,
                result,
            } => {
                if result.is_err() {
                    self.file_list_selection_failed(pane, navigation, revision);
                }
                let applied = self.panes[pane.index()].generation == navigation.0
                    && Arc::ptr_eq(&view, self.panes[pane.index()].display_index())
                    && result.is_ok();
                self.copy_selection_result(pane, navigation, revision, applied);
                let p = &mut self.panes[pane.index()];
                let same_view = Arc::ptr_eq(&view, p.display_index());
                if p.generation == navigation.0
                    && let Some(a) = &mut p.archive
                    && a.selection_pending
                    && a.selection_revision == revision
                {
                    a.selection_pending = false;
                    p.selection_preview
                        .retain(|pending| pending.revision > revision);
                    match result {
                        Ok(result) if same_view => {
                            self.retired
                                .push(Box::new(mem::replace(&mut a.selected, result.selected)));
                            a.totals = result.totals;
                        }
                        Ok(result) => {
                            self.retired.push(Box::new(result));
                            p.set_error("Selection view changed. Try again.", &mut self.retired);
                        }
                        Err(error) => p.set_error(error.to_string(), &mut self.retired),
                    }
                } else {
                    self.retired.push(Box::new(result));
                }
                // A stale response may be the final strong owner of a large index.
                self.retired.push(Box::new(view));
            }
            Event::ArchiveAvailability(formats) => self.archive_formats = formats,
            Event::ArchiveProgress {
                pane,
                navigation,
                staged,
            } => self.archive_progress(pane, navigation, staged),
            Event::ArchiveSessionsFull {
                pane,
                navigation,
                sessions,
                retry,
            } => self.archive_sessions_full(pane, navigation, sessions, retry),
            Event::ArchiveListed {
                pane,
                navigation,
                result,
            } => {
                if navigation.0 != self.panes[pane.index()].generation {
                    self.retired.push(Box::new(result));
                    return None;
                }
                // A listing that came back empty because the archive's names are
                // encrypted: its password is asked for once the listing is shown.
                let names_locked = result
                    .as_ref()
                    .ok()
                    .filter(|listing| listing.entries.is_empty())
                    .and_then(|listing| listing.location.mount.as_ref())
                    .is_some_and(|mount| mount.listing_locked() && !mount.unlocked());
                self.end_opening(pane);
                let p = &mut self.panes[pane.index()];
                p.loading = false;
                p.archive_request = false;
                match result {
                    Err(error) => {
                        p.sort = p.committed_sort;
                        let mut leaving = false;
                        let mut enclosing = false;
                        p.archive_history_request = None;
                        let saved = p.archive_resume.take();
                        // A saved archive location that a native folder could not open
                        // (Back, Go to file) leaves the native folder shown: it is read
                        // again, so that the backend serves it as before, and a notice
                        // keeps the reason, which that re-read clears from the panel.
                        let native = saved.is_some()
                            && p.archive.is_none()
                            && p.successful_generation.is_some()
                            && error.kind != std::io::ErrorKind::Interrupted;
                        if native {
                            p.refresh_hint = true;
                        }
                        self.retired.push(Box::new(saved));
                        if let Some(a) = &mut p.archive {
                            a.wanted_cursor = None;
                            let requested = a.requested.take();
                            let reopening = matches!(a.recovery, Recovery::Reopening);
                            // A recovery may hold an enclosing archive's session.
                            self.retired.push(Box::new(mem::take(&mut a.recovery)));
                            // A cancelled request says nothing about the session.
                            if error.kind != std::io::ErrorKind::Interrupted {
                                if let Some(parent) =
                                    requested.as_ref().and_then(ArchiveLocation::parent)
                                {
                                    // An archive inside an archive is not re-opened, and
                                    // its stopped session is not entered again.
                                    let failed = requested.as_ref().and_then(|r| r.mount_id());
                                    history::forget_inner_sessions(
                                        &mut p.history,
                                        &mut p.history_index,
                                        |location| location.mount_id() == failed,
                                        &mut self.retired,
                                    );
                                    a.recovery = Recovery::Parent(parent);
                                    enclosing = true;
                                } else if let Some(requested) = &requested {
                                    if let Some(session) = a.native_session(&p.history) {
                                        self.archive_sessions.released(pane, session);
                                    }
                                    a.forget_session(&mut p.history, &mut self.retired);
                                    a.recovery = Recovery::Reopen(ArchiveLocation::unavailable(
                                        requested.address.clone(),
                                    ));
                                } else if reopening {
                                    a.recovery = Recovery::Leave;
                                    leaving = true;
                                }
                            }
                            self.retired.push(Box::new(requested));
                        }
                        p.set_error(error.to_string(), &mut self.retired);
                        if leaving || enclosing {
                            self.desktop.report(
                                "Archive unavailable",
                                format!(
                                    "{}\n\nThe panel returns to the folder that contains the archive.",
                                    crate::error_presentation::describe(&error)
                                ),
                            );
                        } else if native {
                            self.desktop.report(
                                "Archive unavailable",
                                crate::error_presentation::describe(&error),
                            );
                        }
                    }
                    Ok(listing) => {
                        self.retired.push(Box::new(p.network.take()));
                        let wanted_cursor = p
                            .archive
                            .as_mut()
                            .and_then(|archive| archive.wanted_cursor.take())
                            .and_then(|wanted| {
                                listing.entries.iter().find(|entry| {
                                    entry
                                        .virtual_node
                                        .as_ref()
                                        .is_some_and(|node| node.id == wanted)
                                })
                            })
                            .map(|entry| entry.id);
                        p.committed_sort = p.sort;
                        let same = p.archive.as_ref().is_some_and(|a| {
                            a.location.mount_id() == listing.location.mount_id()
                                && a.location.directory == listing.location.directory
                        });
                        // A saved location that is still being opened level by level is
                        // recorded once it is reached (`resume_archive`).
                        if p.archive_resume
                            .as_ref()
                            .is_none_or(|resume| resume.reached(&listing.location))
                        {
                            self.retired.push(Box::new(p.archive_resume.take()));
                            let requested = p.archive_history_request.take();
                            p.record_history(
                                HistoryEntry::Archive(listing.location.clone()),
                                requested,
                                &mut self.retired,
                            );
                        }
                        if let Some(archive) = &mut p.archive {
                            if archive.location.mount_id() != listing.location.mount_id() {
                                self.retired.push(Box::new(mem::take(&mut archive.sizes)));
                            }
                            if !same {
                                self.retired
                                    .push(Box::new(mem::take(&mut archive.selected)));
                                archive.totals = ArchiveSelectionTotals::default();
                            }
                            archive.selection_pending = false;
                            self.retired.push(Box::new(archive.requested.take()));
                            self.retired
                                .push(Box::new(mem::take(&mut archive.recovery)));
                            self.retired.push(Box::new(mem::replace(
                                &mut archive.location,
                                listing.location,
                            )));
                        } else {
                            // The native folder that holds the archive, which `..` returns
                            // to: also after Back, a Hot Path or Go to file from elsewhere.
                            let origin = listing
                                .location
                                .outermost()
                                .address
                                .container
                                .parent()
                                .map_or_else(|| p.path.clone(), Path::to_owned);
                            p.archive = Some(ArchivePane {
                                sizes: Default::default(),
                                origin,
                                location: listing.location,
                                wanted_cursor: None,
                                selected: Arc::default(),
                                selected_positions: Default::default(),
                                totals: ArchiveSelectionTotals::default(),
                                selection_pending: false,
                                selection_revision: SelectionRevision(0),
                                requested: None,
                                recovery: Recovery::None,
                                opening: None,
                            });
                        }
                        self.retired
                            .push(Box::new(mem::replace(&mut p.entries, listing.entries)));
                        self.retired
                            .push(Box::new(mem::replace(&mut p.index, listing.index)));
                        self.retired.push(Box::new(p.pending.take()));
                        self.retired.push(Box::new(mem::take(&mut p.selected)));
                        self.retired.push(Box::new(mem::take(&mut p.streaming)));
                        self.retired.push(Box::new(mem::take(&mut p.metadata)));
                        p.folder_column_samples = SizeColumnSamples::default();
                        self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
                        p.selection_preview.clear();
                        p.selection_rebase_pending = false;
                        p.size_scan = None;
                        p.cancelled_size_scan = None;
                        self.retired.push(Box::new(p.size_scan_scope.take()));
                        self.retired
                            .push(Box::new(mem::take(&mut p.entry_size_scans)));
                        p.successful_generation = Some(navigation);
                        p.cursor = wanted_cursor.or_else(|| {
                            p.cursor.filter(|id| same && p.index.by_id.contains_key(id))
                        });
                        p.parent_cursor = p.cursor.is_none();
                        p.set_error("", &mut self.retired);
                    }
                }
                // An open refused for want of a slot offers to close an archive.
                self.offer_archive_close(pane, navigation);
                if names_locked
                    && let Some(location) = self.panes[pane.index()]
                        .archive
                        .as_ref()
                        .map(|archive| archive.location.clone())
                {
                    self.listing_needs_archive_password(pane, &location);
                }
            }
            other => return Some(other),
        }
        None
    }
    pub(super) fn archive_up(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(archive) = &self.panes[pane.index()].archive else {
            return false;
        };
        let mut location = archive.location.clone();
        if location.trail.len() > 1 {
            location.trail.pop();
            let steps = location.address.step_count();
            location.address.truncate_steps(steps.saturating_sub(1));
            location.directory = location.trail.last().map_or(1, |v| v.0);
            self.archive_navigate(pane, ArchiveAction::List(location), send)
        } else if let Some(parent) = location.parent() {
            // The root of an archive inside an archive: its enclosing folder, with the
            // cursor on it (`cursor_toward`).
            self.archive_navigate(pane, ArchiveAction::List(parent.location), send)
        } else {
            self.leave_archive(pane, send)
        }
    }
    /// Return to the native folder that contains the archive, cursor on the archive.
    fn leave_archive(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(archive) = &self.panes[pane.index()].archive else {
            return false;
        };
        let origin = archive.origin.clone();
        let container = archive.location.address.container.clone();
        if !self.navigate(pane, origin, None, send) {
            return false;
        }
        let p = &mut self.panes[pane.index()];
        if let Some(request) = &mut p.pending {
            request.wanted_cursor = Some(container);
        }
        p.parent_cursor = false;
        true
    }
    /// Carry out the recovery of a failed mounted listing (see `Recovery`). A refused
    /// request stays owed and is retried with the other background refreshes.
    pub(super) fn recover_archive(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &self.panes[pane.index()];
        let Some(archive) = p.archive.as_ref().filter(|_| !p.loading) else {
            return;
        };
        match &archive.recovery {
            Recovery::Reopen(location) => {
                let location = location.clone();
                if self.archive_navigate_impl(
                    pane,
                    ArchiveAction::List(location),
                    Reload::Background,
                    send,
                ) && let Some(archive) = &mut self.panes[pane.index()].archive
                {
                    archive.recovery = Recovery::Reopening;
                }
            }
            Recovery::Leave => {
                if self.leave_archive(pane, send)
                    && let Some(archive) = &mut self.panes[pane.index()].archive
                {
                    archive.recovery = Recovery::None;
                }
            }
            Recovery::Parent(parent) => {
                let ArchiveParent {
                    location, member, ..
                } = parent.clone();
                if self.archive_navigate_impl(
                    pane,
                    ArchiveAction::List(location),
                    Reload::Background,
                    send,
                ) && let Some(archive) = &mut self.panes[pane.index()].archive
                {
                    archive.wanted_cursor = Some(member);
                }
            }
            Recovery::None | Recovery::Reopening => {}
        }
    }
    pub(super) fn archive_open(
        &mut self,
        pane: PaneId,
        row: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if row == 0 {
            return self.archive_up(pane, send);
        }
        let p = &self.panes[pane.index()];
        let Some(node) = p
            .display_entry(row - 1)
            .and_then(|e| e.virtual_node.as_ref())
            .cloned()
        else {
            return false;
        };
        let Some(archive) = &p.archive else {
            return false;
        };
        if node.kind != EntryKind::Directory {
            if let Some(format) = self.member_archive_format(&node) {
                return self.open_member_archive(pane, *node, format, send);
            }
            self.desktop.report(
                "Archive member",
                "Use F3 to view this member or F5 to copy it to a native folder. External launch inside an archive is unavailable.".into(),
            );
            return false;
        }
        let mut location = archive.location.clone();
        location.directory = node.id.entry;
        location.trail.push((node.id.entry, node.display_name));
        location.address.components.push(node.raw_name.to_vec());
        if let Err(error) = location.address.validate() {
            self.desktop.report("Archive location limit", error.into());
            return false;
        }
        self.archive_navigate(pane, ArchiveAction::List(location), send)
    }
    /// Enter on an archive member whose name is an enabled archive: open it as an
    /// archive inside this archive, listed from its root in this pane.
    pub(super) fn open_member_archive(
        &mut self,
        pane: PaneId,
        node: ArchiveNode,
        format: ArchiveFormat,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(location) = self.panes[pane.index()]
            .archive
            .as_ref()
            .map(|archive| archive.location.clone())
        else {
            return false;
        };
        self.open_member_archive_in(pane, location, node, format, send)
    }

    /// Open `node`, a member of the folder `location` of `pane`'s archive, as an archive
    /// inside it. The password dialog passes the folder that Enter named.
    pub(super) fn open_member_archive_in(
        &mut self,
        pane: PaneId,
        location: ArchiveLocation,
        node: ArchiveNode,
        format: ArchiveFormat,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(archive) = &self.panes[pane.index()].archive else {
            return false;
        };
        let refusal = if location.mount_id() != Some(node.id.mount) {
            Some((
                "Archive unavailable",
                "This archive is no longer open. Open it again to open the archives inside it."
                    .to_owned(),
            ))
        } else if !node.readable {
            Some(("Archive member", node.unreadable_reason()))
        } else if location.depth() >= MAX_ARCHIVE_DEPTH {
            Some(("Archive nesting limit", archive_depth_message()))
        } else {
            None
        };
        if let Some((title, message)) = refusal {
            self.desktop.report(title, message);
            return false;
        }
        // An encrypted inner archive needs the enclosing archive's password first.
        if let Some(mount) = self.archive_password_needed(pane, &node) {
            let name = midnight_core::visible_name(&node.display_name).into_owned();
            self.ask_archive_password(
                pane,
                mount,
                format!("Enter its password to open {name}."),
                super::archive_password::PasswordAction::OpenMember {
                    pane,
                    location: Box::new(location),
                    node: Box::new(node),
                    format,
                },
            );
            return true;
        }
        // An inner archive this pane still remembers is entered through its session.
        let p = &self.panes[pane.index()];
        if let Some(root) = ArchivePane::remembered_inner_root(&p.history, node.id) {
            return self.archive_navigate(pane, ArchiveAction::List(root), send);
        }
        let shown = archive.shown_sessions();
        let p = &mut self.panes[pane.index()];
        history::forget_inner_sessions(
            &mut p.history,
            &mut p.history_index,
            |location| {
                location
                    .mount_id()
                    .is_some_and(|mount| !shown.contains(&mount))
            },
            &mut self.retired,
        );
        let (name, size) = (node.display_name.clone(), node.size);
        let parent = ArchiveParent {
            location,
            member: node.id,
            name: node.display_name,
            raw_name: node.raw_name.to_vec(),
        };
        if !self.archive_navigate(pane, ArchiveAction::OpenMember { parent, format }, send) {
            return false;
        }
        let p = &mut self.panes[pane.index()];
        if let Some(archive) = &mut p.archive {
            archive.opening = Some(Opening {
                navigation: NavigationId(p.generation),
                name,
                size,
                status: None,
            });
        }
        true
    }
    pub(crate) fn native_action_allowed(&mut self, pane: PaneId) -> bool {
        if self.panes[pane.index()].native_capabilities() {
            return true;
        }
        self.desktop.report(
            "Filesystem action unavailable",
            "Open a native folder or network share before using this action.".into(),
        );
        false
    }
    pub(crate) fn refresh_location(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.panes[pane.index()].network_pending.is_some() {
            let Some(action) = self.panes[pane.index()].pending_network_browse() else {
                return false;
            };
            return self.network_navigate(pane, action, send);
        }
        if let Some(network) = &self.panes[pane.index()].network {
            return self.network_navigate(
                pane,
                NetworkAction::Refresh(network.address.clone()),
                send,
            );
        }
        if let Some(a) = &self.panes[pane.index()].archive {
            return self.archive_navigate(pane, ArchiveAction::List(a.location.clone()), send);
        }
        if self.request_native_tree_refresh(pane, send) {
            return true;
        }
        self.navigate(
            pane,
            self.panes[pane.index()].display_path().to_owned(),
            None,
            send,
        )
    }
    pub(super) fn archive_select_row(
        &mut self,
        pane: PaneId,
        row: usize,
        control: bool,
        shift: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if row == 0 {
            self.focus_display_row(pane, row);
            return;
        }
        let p = &self.panes[pane.index()];
        if p.loading || p.search_view_waiting() {
            return;
        }
        let Some((entry, kind, size)) = p
            .display_entry(row - 1)
            .map(|e| (e.id, e.kind, e.metadata.size))
        else {
            return;
        };
        // The anchor while the cursor stayed where it left it; else the cursor's row;
        // else the clicked row.
        let shown = |id: EntryId| p.display_index().by_id.get(&id).copied();
        let first = p
            .selection_anchor_id
            .filter(|_| p.anchor_current())
            .and_then(shown)
            .or_else(|| p.cursor.and_then(shown))
            .unwrap_or(row - 1);
        let first_id = p.display_entry(first).map(|e| e.id);
        self.focus_display_row(pane, row);
        if shift {
            let p = &mut self.panes[pane.index()];
            p.selection_anchor_id = first_id.or(p.selection_anchor_id);
            p.keep_anchor();
            self.archive_bulk_selection(
                pane,
                ArchiveSelectionAction::Range {
                    first: first.min(row - 1),
                    last: first.max(row - 1),
                    additive: control,
                },
                send,
            );
        } else {
            let p = &mut self.panes[pane.index()];
            p.selection_anchor_id = Some(entry);
            p.keep_anchor();
            if control
                && p.archive
                    .as_ref()
                    .is_some_and(|a| Arc::strong_count(&a.selected) > 1)
            {
                self.archive_bulk_selection(pane, ArchiveSelectionAction::Toggle(entry), send);
                return;
            }
            if control
                && let Some(a) = &mut p.archive
                && !a.selection_pending
            {
                let Some(revision) = a.selection_revision.0.checked_add(1).map(SelectionRevision)
                else {
                    p.set_error(
                        "Selection request identities are exhausted.",
                        &mut self.retired,
                    );
                    return;
                };
                let selected = Arc::make_mut(&mut a.selected);
                let removed = selected.remove(&entry);
                if !removed {
                    selected.insert(entry);
                }
                a.totals.change(kind, size, !removed);
                a.selection_revision = revision;
            }
        }
    }
    pub(super) fn archive_bulk_selection(
        &mut self,
        pane: PaneId,
        action: ArchiveSelectionAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        let Some(a) = &p.archive else {
            return false;
        };
        if a.selection_pending || p.loading || p.search_view_waiting() {
            return false;
        }
        let Some(revision) = a.selection_revision.0.checked_add(1).map(SelectionRevision) else {
            self.panes[pane.index()].set_error(
                "Selection request identities are exhausted.",
                &mut self.retired,
            );
            return false;
        };
        let preview_action = match &action {
            ArchiveSelectionAction::Bulk(operation) => Some(SelectionAction::Bulk(*operation)),
            ArchiveSelectionAction::Ranges { ranges, mode } => Some(SelectionAction::Ranges {
                ranges: ranges.clone(),
                mode: *mode,
            }),
            _ => None,
        };
        let preview = preview_action
            .map(|action| selection_preview::PendingSelection::new(p, revision, action));
        if !send(Command::ArchiveSelect {
            pane,
            navigation: NavigationId(p.generation),
            revision,
            entries: p.entries.clone(),
            view: p.display_index().clone(),
            selected: a.selected.clone(),
            totals: a.totals,
            action,
        }) {
            self.panes[pane.index()]
                .set_error("Selection queue is busy. Try again.", &mut self.retired);
            return false;
        }
        if let Some(a) = &mut self.panes[pane.index()].archive {
            a.selection_pending = true;
            a.selection_revision = revision;
        }
        if let Some(preview) = preview {
            self.panes[pane.index()].selection_preview.push(preview);
        }
        true
    }
}
impl Pane {
    pub(super) fn archive_selection_summary(&self) -> Option<(usize, usize, Option<u64>, bool)> {
        if let Some(publication) = &self.tree.publication {
            return (!publication.selection.is_empty() || !self.tree.preview.is_empty()).then_some(
                (
                    publication.selection.file_count(),
                    publication.selection.folder_count(),
                    publication.selection.file_bytes(),
                    !self.tree.preview.is_empty(),
                ),
            );
        }
        let archive = self.archive.as_ref()?;
        (!archive.selected.is_empty() || archive.selection_pending).then_some((
            archive.totals.files(),
            archive.totals.folders(),
            archive.totals.file_bytes(),
            archive.selection_pending,
        ))
    }
    pub(crate) fn native_capabilities(&self) -> bool {
        self.archive.is_none()
            && !self.archive_request
            && self.network.is_none()
            && !self.network_request
    }
    /// Displayed archive authority only; pending navigation does not retarget visible rows.
    pub(crate) fn thumbnail_archive_location(&self) -> Option<&ArchiveLocation> {
        self.archive.as_ref().map(|archive| &archive.location)
    }
    pub(crate) fn display_location_text(&self) -> String {
        if let Some(text) = self.pending_network_text() {
            return text.to_owned();
        }
        if let Some(network) = &self.network
            && self.pending.is_none()
        {
            return network.address.display.clone();
        }
        self.archive.as_ref().map_or_else(
            || self.display_path().to_string_lossy().into_owned(),
            |a| a.location.display_text(),
        )
    }
    pub(crate) fn selected_count(&self) -> usize {
        if self.network_hides_rows() {
            return 0;
        }
        if let Some(publication) = &self.tree.publication {
            return publication.selection.len();
        }
        self.archive
            .as_ref()
            .map_or_else(|| self.selected.len(), |a| a.selected.len())
    }
}
