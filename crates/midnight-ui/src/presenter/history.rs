//! Back and Forward of one tab: native folders and folders inside archives (also inside
//! archives inside archives), in the order the pane showed them. Entering, leaving and
//! moving inside an archive are navigations like any other. While the pane shows an
//! archive, the entries of that archive keep its sessions; once it leaves the archive they
//! keep only their saved addresses (`ArchiveLocation::saved_address`), and returning to
//! one opens the archive, and the archives inside it, again.
use super::*;

#[derive(Clone, Debug)]
pub(crate) enum HistoryEntry {
    Native(PathBuf),
    Archive(ArchiveLocation),
}

/// Native entries compare with their paths, so that a native history reads as one.
impl PartialEq<PathBuf> for HistoryEntry {
    fn eq(&self, other: &PathBuf) -> bool {
        matches!(self, Self::Native(path) if path == other)
    }
}

impl HistoryEntry {
    /// The native folder of a native entry.
    pub(crate) fn native(&self) -> Option<&Path> {
        match self {
            Self::Native(path) => Some(path),
            Self::Archive(_) => None,
        }
    }

    /// The location as the history lists show it.
    pub(crate) fn display_text(&self) -> String {
        match self {
            Self::Native(path) => path.to_string_lossy().into_owned(),
            Self::Archive(location) => location.display_text(),
        }
    }

    /// Whether `listed`, a location the pane just showed, is this entry's location.
    fn same_place(&self, listed: &Self) -> bool {
        match (self, listed) {
            (Self::Native(left), Self::Native(right)) => left == right,
            (Self::Archive(left), Self::Archive(right)) => {
                match (left.mount_id(), right.mount_id()) {
                    (Some(a), Some(b)) => a == b && left.directory == right.directory,
                    _ => left.saved_address().same_location(&right.saved_address()),
                }
            }
            _ => false,
        }
    }
}

/// The archive locations of `history`.
pub(super) fn archives(history: &[HistoryEntry]) -> impl Iterator<Item = &ArchiveLocation> {
    history.iter().filter_map(|entry| match entry {
        HistoryEntry::Archive(location) => Some(location),
        HistoryEntry::Native(_) => None,
    })
}

pub(super) fn archives_mut(
    history: &mut [HistoryEntry],
) -> impl Iterator<Item = &mut ArchiveLocation> {
    history.iter_mut().filter_map(|entry| match entry {
        HistoryEntry::Archive(location) => Some(location),
        HistoryEntry::Native(_) => None,
    })
}

/// Let go of the archive sessions that `history` holds: each entry keeps its saved
/// address, and returning to it opens its archive again.
pub(super) fn release_sessions(history: &mut [HistoryEntry], retired: &mut Vec<DeferredDrop>) {
    for location in archives_mut(history) {
        if location.mount.is_some() {
            let saved = ArchiveLocation::unavailable(location.saved_address());
            retired.push(Box::new(mem::replace(location, saved)));
        }
    }
}

/// Forget the entries of archives inside archives that `left` names. Each such session
/// keeps a worker of the bounded nested allowance, so the folders of archives the pane
/// has left go before another one opens. The position stays on the same entry, or on
/// the nearest earlier one that remains.
pub(super) fn forget_inner_sessions(
    history: &mut Vec<HistoryEntry>,
    index: &mut Option<usize>,
    left: impl Fn(&ArchiveLocation) -> bool,
    retired: &mut Vec<DeferredDrop>,
) {
    let current = *index;
    let mut kept = None;
    for (position, entry) in mem::take(history).into_iter().enumerate() {
        if let HistoryEntry::Archive(location) = &entry
            && location.parent().is_some()
            && left(location)
        {
            retired.push(Box::new(entry));
            continue;
        }
        if current.is_some_and(|current| position <= current) {
            kept = Some(history.len());
        }
        history.push(entry);
    }
    *index = current.and(kept.or((!history.is_empty()).then_some(0)));
}

/// A saved archive location that the backend opened only part of the way: the archives
/// inside it that need a password are opened here, where the password is asked for.
pub(super) struct ArchiveResume {
    target: ArchiveAddress,
    /// The listing from which the next level was last attempted.
    tried: Option<NavigationId>,
}

impl ArchiveResume {
    pub(super) fn new(target: ArchiveAddress) -> Self {
        Self {
            target,
            tried: None,
        }
    }

    pub(super) fn reached(&self, listed: &ArchiveLocation) -> bool {
        self.target.same_location(&listed.saved_address())
    }

    /// Whether `action`, sent while `shown` is listed, goes on toward the target: a folder
    /// or archive on its way, or the shown folder again (after its password).
    pub(super) fn continued_by(
        &self,
        action: &ArchiveAction,
        shown: Option<&ArchiveLocation>,
    ) -> bool {
        let Some(shown) = shown else {
            return false;
        };
        let next = match action {
            ArchiveAction::List(location) if location.mount.is_some() => location.saved_address(),
            ArchiveAction::OpenMember { parent, format } => {
                let mut address = parent.location.saved_address();
                address.nested.push(NestedArchiveAddress {
                    member: parent.raw_name.clone(),
                    format: *format,
                    components: Vec::new(),
                });
                address
            }
            _ => return false,
        };
        let shown = shown.saved_address();
        let (target, next_steps) = (self.target.steps(), next.steps());
        self.target.same_archive(&next)
            && next_steps.len() >= shown.step_count()
            && target.starts_with(&next_steps)
    }
}

impl Pane {
    /// Record the location this pane just showed: at `requested`, the entry that Back,
    /// Forward or a history list went to, else after the current entry unless it is this
    /// location already. A new location drops the entries after the current one. An
    /// archive entry takes the listed location, which keeps its session.
    pub(super) fn record_history(
        &mut self,
        entry: HistoryEntry,
        requested: Option<usize>,
        retired: &mut Vec<DeferredDrop>,
    ) {
        let archive = matches!(entry, HistoryEntry::Archive(_));
        let current = requested
            .filter(|index| *index < self.history.len())
            .or_else(|| {
                self.history_index.filter(|index| {
                    self.history
                        .get(*index)
                        .is_some_and(|e| e.same_place(&entry))
                })
            });
        if let Some(index) = current {
            self.history_index = Some(index);
            if archive {
                retired.push(Box::new(mem::replace(&mut self.history[index], entry)));
            } else {
                retired.push(Box::new(entry));
            }
            return;
        }
        let retained = self.history_index.map_or(0, |index| index + 1);
        if retained < self.history.len() {
            retired.push(Box::new(self.history.split_off(retained)));
        }
        self.history.push(entry);
        if self.history.len() > MAX_HISTORY {
            retired.push(Box::new(self.history.remove(0)));
        }
        self.history_index = Some(self.history.len() - 1);
    }

    /// The archive session opened from a native file that this tab holds.
    pub(super) fn archive_native_session(&self) -> Option<u64> {
        self.archive.as_ref()?.native_session(&self.history)
    }

    /// Whether this tab holds archive session `session`.
    pub(crate) fn holds_archive_session(&self, session: u64) -> bool {
        self.archive
            .as_ref()
            .is_some_and(|archive| archive.holds_session(&self.history, session))
    }
}

impl Presenter {
    pub(crate) fn history(
        &mut self,
        pane: PaneId,
        delta: isize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.panes[pane.index()].network.is_some() {
            return self.network_history(pane, delta, send);
        }
        if delta == -1
            && let Some(previous) = &self.panes[pane.index()].network_return
            && previous.root == self.panes[pane.index()].display_path()
        {
            return self.network_navigate(
                pane,
                NetworkAction::Browse(previous.context.address.clone()),
                send,
            );
        }
        let p = &self.panes[pane.index()];
        let Some(index) = p
            .history_index
            .and_then(|i| i.checked_add_signed(delta))
            .filter(|i| *i < p.history.len())
        else {
            return false;
        };
        self.history_go(pane, index, send)
    }

    /// Show history entry `index` of `pane`. Leaving an archive for a native folder that
    /// holds it puts the cursor on the archive, or on the folder that leads to it.
    pub(super) fn history_go(
        &mut self,
        pane: PaneId,
        index: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        let Some(entry) = p.history.get(index).cloned() else {
            return false;
        };
        match entry {
            HistoryEntry::Native(path) => {
                let container = p.archive.as_ref().map(|a| -> PathBuf {
                    a.location
                        .outermost()
                        .address
                        .container
                        .components()
                        .collect()
                });
                if !self.navigate(pane, path.clone(), Some(index), send) {
                    return false;
                }
                let child = container.and_then(|container| {
                    container
                        .ancestors()
                        .find(|child| child.parent() == Some(path.as_path()))
                        .map(Path::to_owned)
                });
                if let Some(child) = child {
                    let p = &mut self.panes[pane.index()];
                    if let Some(request) = &mut p.pending {
                        request.wanted_cursor = Some(child);
                    }
                    p.parent_cursor = false;
                }
                true
            }
            HistoryEntry::Archive(location) => {
                if !self.archive_navigate(pane, ArchiveAction::List(location), send) {
                    return false;
                }
                self.panes[pane.index()].archive_history_request = Some(index);
                true
            }
        }
    }

    /// Go on opening a saved archive location that the backend opened only part of the
    /// way (an archive inside it, or its names, need a password): list the next folder,
    /// or open the next archive, which asks for its password first. Once per listing,
    /// and never while the password dialog is open.
    pub(super) fn resume_archive(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &self.panes[pane.index()];
        let (Some(resume), Some(archive), Some(listed)) =
            (&p.archive_resume, &p.archive, p.successful_generation)
        else {
            return;
        };
        let shown = &archive.location;
        if p.loading
            || self.archive_password.open
            || resume.tried == Some(listed)
            || shown
                .mount
                .as_ref()
                .is_none_or(|mount| mount.listing_locked() && !mount.unlocked())
        {
            return;
        }
        let saved = shown.saved_address();
        let (done, steps) = (saved.steps(), resume.target.steps());
        let levels = resume.target.nested.len() + 1;
        // The next folder (no decoder) or archive (its decoder), by raw name.
        let next: Option<(Vec<u8>, Option<ArchiveFormat>)> = steps
            .get(done.len())
            .filter(|_| saved.same_archive(&resume.target) && steps.starts_with(&done))
            .map(|step| match *step {
                ArchiveStep::Folder(name) => (name.to_vec(), None),
                ArchiveStep::Archive(name, format) => (name.to_vec(), Some(format)),
            });
        let level = done
            .iter()
            .filter(|step| matches!(step, ArchiveStep::Archive(..)))
            .count();
        let found = next.as_ref().and_then(|(name, format)| {
            p.entries
                .iter()
                .filter_map(|entry| entry.virtual_node.as_deref())
                .find(|node| {
                    (node.kind == EntryKind::Directory) == format.is_none()
                        && node.raw_name.as_ref() == name.as_slice()
                })
                .cloned()
        });
        let shown = shown.clone();
        let requested = p.archive_history_request;
        let (Some((name, format)), Some(node)) = (next.clone(), found) else {
            // The rest of the saved location is gone, or the pane went elsewhere: it
            // stays where it is, which history now records.
            let missing = next.map(|(name, _)| String::from_utf8_lossy(&name).into_owned());
            let p = &mut self.panes[pane.index()];
            self.retired.push(Box::new(p.archive_resume.take()));
            p.archive_history_request = None;
            p.record_history(HistoryEntry::Archive(shown), requested, &mut self.retired);
            if let Some(name) = missing {
                self.desktop.report(
                    "Saved location unavailable",
                    format!(
                        "{name} is no longer in archive {} of {levels} on the saved path. \
                         The panel shows the folder that held it.",
                        level + 1
                    ),
                );
            }
            return;
        };
        if let Some(resume) = &mut self.panes[pane.index()].archive_resume {
            resume.tried = Some(listed);
        }
        match format {
            None => {
                let mut child = shown;
                child.directory = node.id.entry;
                child.trail.push((node.id.entry, node.display_name));
                child.address.components.push(name);
                self.archive_navigate(pane, ArchiveAction::List(child), send);
            }
            Some(format) => {
                self.open_member_archive_in(pane, shown, node, format, send);
            }
        }
    }
}
