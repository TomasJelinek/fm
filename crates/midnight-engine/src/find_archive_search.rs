//! Search inside archives. An archive file met by the native walk is opened through
//! the archive provider, and its members are matched like native entries: names,
//! metadata expressions, modification times and (streamed through the provider)
//! content. Nothing limits archives as such: every archive and every member is searched
//! until the search ends, and Stop reaches the provider's work (opening, listing and
//! member reads) through the search's cancellation. The search's node, depth, result,
//! memory and content budgets apply to members as to native entries. Nested archives
//! are not opened.
use super::*;

/// An open archive: its provider session and the address of its root folder.
pub(crate) struct OpenedArchive {
    pub(crate) mount: Arc<dyn ArchiveMount>,
    pub(crate) root: ArchiveAddress,
}

/// Opens archives for Find. Every call runs on the search worker.
pub(crate) trait ArchiveSearch: Send + Sync {
    /// None when no installed provider handles this file's name.
    fn open(
        &self,
        archive: &FindEntry,
        cancel: &Cancellation,
    ) -> Option<Result<OpenedArchive, FsError>>;
    /// Ends the session on this worker.
    fn release(&self, mount: Arc<dyn ArchiveMount>) -> Result<(), FsError>;
}

/// A cheap name check before any provider work: a built-in archive suffix.
pub(super) fn eligible(entry: &FindEntry) -> bool {
    archive_format(&entry.path).is_some()
}

/// The provider's root folder ID (`ArchiveLocation::directory`).
const ROOT: u64 = 1;

/// Searches one archive. False when the whole search must end.
pub(super) fn search(
    archives: &dyn ArchiveSearch,
    container: &mut dyn FindSource,
    filter: &predicate::Predicate,
    content: &mut Option<content::PreparedContent>,
    work: &mut Work<'_>,
    accepted_at: SystemTime,
) -> bool {
    let archive = container.entry().clone();
    let opened = match archives.open(&archive, work.cancel) {
        None => return true,
        Some(Ok(opened)) => opened,
        Some(Err(error)) => {
            work.error(error);
            return !work.stopped();
        }
    };
    work.progress.archives = work.progress.archives.saturating_add(1);
    // The archive must still be the file the walk found.
    let mut keep_going = match container.revalidate(work.cancel) {
        Ok(()) => walk(&opened, &archive, filter, content, work, accepted_at),
        Err(error) => {
            work.error(error);
            !work.stopped()
        }
    };
    if let Err(error) = archives.release(opened.mount) {
        work.error(error);
        keep_going &= !work.stopped();
    }
    keep_going
}

/// A folder still to list: provider ID, raw names from the root, shown path, and the
/// bytes it holds in the retained-memory budget.
type Folder = (u64, Vec<Vec<u8>>, std::path::PathBuf, usize);

/// A waiting folder is charged to the retained-memory budget like a native directory
/// frame, so that budget bounds the folders a huge archive leaves waiting.
fn waiting(
    work: &mut Work<'_>,
    id: u64,
    components: Vec<Vec<u8>>,
    shown: std::path::PathBuf,
) -> Option<Folder> {
    let cost = size_of::<Folder>()
        + components
            .iter()
            .map(|part| part.len() + size_of::<Vec<u8>>())
            .sum::<usize>()
        + shown.as_os_str().as_encoded_bytes().len();
    work.reserve(cost).then_some((id, components, shown, cost))
}

fn walk(
    opened: &OpenedArchive,
    archive: &FindEntry,
    filter: &predicate::Predicate,
    content: &mut Option<content::PreparedContent>,
    work: &mut Work<'_>,
    accepted_at: SystemTime,
) -> bool {
    let Some(root) = waiting(work, ROOT, Vec::new(), archive.path.clone()) else {
        return false;
    };
    let mut folders = vec![root];
    let keep_going = walk_folders(
        &mut folders,
        opened,
        archive,
        filter,
        content,
        work,
        accepted_at,
    );
    // Folders left waiting by a walk that ended early return their bytes.
    work.retained -= folders.iter().map(|folder| folder.3).sum::<usize>();
    keep_going
}

fn walk_folders(
    folders: &mut Vec<Folder>,
    opened: &OpenedArchive,
    archive: &FindEntry,
    filter: &predicate::Predicate,
    content: &mut Option<content::PreparedContent>,
    work: &mut Work<'_>,
    accepted_at: SystemTime,
) -> bool {
    let request = work.request;
    let limits = request.limits;
    let mount = opened.mount.as_ref();
    while let Some((folder, components, shown, cost)) = folders.pop() {
        work.retained -= cost;
        if work.stopped() {
            return false;
        }
        let nodes = match mount.list(folder, work.cancel) {
            Ok(nodes) => nodes,
            Err(error) => {
                work.error(error);
                if work.stopped() {
                    return false;
                }
                continue;
            }
        };
        for node in nodes {
            if work.stopped() {
                return false;
            }
            if !work.visit_node() {
                return false;
            }
            work.progress.archive_members = work.progress.archive_members.saturating_add(1);
            let name = String::from_utf8_lossy(&node.raw_name).into_owned();
            let path = shown.join(&name);
            let entry = member_entry(&node, path, archive);
            let mut matches = filter.matches(&entry)
                && request.modified_within.is_none_or(|maximum| {
                    entry.metadata.modified.is_some_and(|modified| {
                        accepted_at
                            .duration_since(modified)
                            .is_ok_and(|age| age <= maximum)
                    })
                });
            if let Some(content) = content {
                matches &= entry.kind == EntryKind::File;
                // A member that cannot be read, or whose archive's password was never
                // entered (Find's own sessions never ask), is skipped with its reason.
                if matches && (!node.readable || (node.needs_password() && !mount.unlocked())) {
                    work.error(if node.readable {
                        archive_password_required(&entry.path)
                    } else {
                        find_error(
                            &entry.path,
                            io::ErrorKind::Unsupported,
                            &node.unreadable_reason(),
                        )
                    });
                    matches = false;
                }
                if matches {
                    let mut member = Member {
                        mount,
                        id: node.id.entry,
                        entry: entry.clone(),
                        offset: 0,
                    };
                    matches =
                        match content.matches(&mut member, limits, &mut work.progress, work.cancel)
                        {
                            ContentMatch::Match(matches) => matches,
                            ContentMatch::Limited(budget) => {
                                work.progress.limited_files =
                                    work.progress.limited_files.saturating_add(1);
                                work.budget(budget);
                                false
                            }
                            ContentMatch::Failed(error) => {
                                work.error(error);
                                false
                            }
                            ContentMatch::Cancelled => {
                                work.cancelled = true;
                                return false;
                            }
                        };
                }
            }
            if matches {
                let mut folder = opened.root.clone();
                folder.components = components.clone();
                let member = Arc::new(FindArchiveMember {
                    folder,
                    name: node.raw_name.to_vec(),
                    container: archive.clone(),
                });
                let entry = localized_member(entry, &node, work.locale);
                if !work.publish(entry, Some(member)) {
                    return false;
                }
            }
            if node.kind == EntryKind::Directory {
                if components.len() + 1 >= limits.max_depth {
                    work.budget(FindBudget::Depth);
                } else {
                    let mut next = components.clone();
                    next.push(node.raw_name.to_vec());
                    let Some(subfolder) = waiting(work, node.id.entry, next, shown.join(&name))
                    else {
                        return false;
                    };
                    folders.push(subfolder);
                }
            }
            work.progress_if_due();
        }
    }
    true
}

/// A member's facts as a Find entry. Its identity is synthetic: members are never
/// native targets, and every action on them goes back through the archive.
fn member_entry(node: &ArchiveNode, path: std::path::PathBuf, archive: &FindEntry) -> FindEntry {
    let metadata = Entry::from_archive(node.clone()).metadata;
    let mut object = [0; 16];
    object[..8].copy_from_slice(&node.id.entry.to_le_bytes());
    object[8..].copy_from_slice(&archive.identity.object[..8]);
    FindEntry {
        path,
        kind: node.kind,
        metadata,
        identity: TransferIdentity {
            device: u64::MAX,
            object,
        },
        revision: archive.revision,
    }
}

/// Zone-less archive times (ZIP) are shown as stored, never as an invented instant.
fn localized_member(
    mut entry: FindEntry,
    node: &ArchiveNode,
    locale: Option<&dyn LocaleService>,
) -> FindEntry {
    if let Some(
        time @ ArchiveTimestamp::Local {
            year,
            month,
            day,
            hour,
            minute,
            second,
        },
    ) = node.modified
        && time.is_valid()
    {
        entry.metadata.localized_modified = locale
            .and_then(|locale| locale.format_archive_local_time(time))
            .or_else(|| {
                Some(LocalizedDateTime {
                    date: format!("{year:04}-{month:02}-{day:02}"),
                    time: format!("{hour:02}:{minute:02}:{second:02}"),
                })
            });
    } else {
        localize(&mut entry, locale);
    }
    entry
}

/// A member's content, streamed through the provider in the content reader's chunks.
struct Member<'a> {
    mount: &'a dyn ArchiveMount,
    id: u64,
    entry: FindEntry,
    offset: u64,
}

impl FindSource for Member<'_> {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        self.mount.validate()
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
        Err(find_error(
            &self.entry.path,
            io::ErrorKind::InvalidInput,
            "An archive member is not listed as a folder here.",
        ))
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let bytes = self.mount.read(self.id, self.offset, length, cancel)?;
        if bytes.len() > buffer.len() {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidData,
                "The archive provider returned more bytes than requested.",
            ));
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        self.offset += bytes.len() as u64;
        Ok(bytes.len())
    }
}
