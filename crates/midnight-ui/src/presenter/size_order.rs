//! A folder panel sorted by Size follows the folder sizes calculated after its sort:
//! when they change the order of its folders, the backend re-sorts the shown listing
//! (a native folder or an archive folder). The cursor and the selection stay on their
//! items (a same-folder re-sort keeps both, as an explicit sort does). A running
//! calculation re-sorts at most once per interval and its end once more; each check
//! is one pass over the folder rows, which lead every listing.
use super::*;
use std::time::{Duration, Instant};

/// The shortest time between two re-sorts while a calculation is still running.
const RUNNING_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct SizeOrder {
    /// Bumped whenever a calculated native folder size is stored.
    version: u64,
    /// The size state (archive mount and snapshot version, or 0 and `version`) the
    /// shown order was last checked against.
    checked: Option<(u64, u64)>,
    /// When the order was last checked; a running calculation checks (and so
    /// re-sorts) at most once per interval.
    checked_at: Option<Instant>,
}

impl Pane {
    /// A calculated native folder size was stored: a Size sort may need to move it.
    pub(crate) fn folder_size_stored(&mut self) {
        self.size_order.version = self.size_order.version.wrapping_add(1);
    }

    /// The size state a Size sort of this flat listing follows, and whether a
    /// calculation is still running; None when it re-sorts on its own or not now.
    fn size_order_signal(&self) -> Option<((u64, u64), bool)> {
        if self.sort.column != SortColumn::Size
            || self.loading
            || self.pending.is_some()
            || self.tree_view().is_some()
            || self.network.is_some()
            || self.network_pending.is_some()
        {
            return None;
        }
        if let Some(archive) = &self.archive {
            if self.archive_request {
                return None;
            }
            let (mount, version, running) = archive.sizes.size_order_signal();
            return Some(((mount, version), running));
        }
        self.successful_generation?;
        let running = self.size_scan.is_some() || !self.entry_size_scans.is_empty();
        Some(((0, self.size_order.version), running))
    }

    /// Whether this listing, sorted by Size, is out of the order its current
    /// calculated folder sizes give.
    fn size_order_stale(&self) -> bool {
        let folders = self
            .entries
            .iter()
            .take_while(|entry| {
                matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
            })
            .count();
        let folders = &self.entries[..folders];
        let sorted = match &self.archive {
            Some(archive) => midnight_core::is_sorted_with_sizes(folders, self.sort, |entry| {
                archive.sizes.sort_size(entry)
            }),
            None => midnight_core::is_sorted_with_sizes(folders, self.sort, |entry| {
                self.folder_sizes
                    .get(&entry.path)
                    .and_then(FolderSize::sort_bytes)
            }),
        };
        !sorted
    }
}

impl Presenter {
    /// Re-sort a panel whose calculated folder sizes changed its Size order.
    pub(super) fn flush_size_order(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            if self.feed(pane).is_some() || self.finds.feed_builds[pane.index()].is_some() {
                continue;
            }
            let p = &self.panes[pane.index()];
            let Some((signal, running)) = p.size_order_signal() else {
                continue;
            };
            if p.size_order.checked == Some(signal)
                || (running
                    && p.size_order
                        .checked_at
                        .is_some_and(|last| last.elapsed() < RUNNING_INTERVAL))
            {
                continue;
            }
            let stale = p.size_order_stale();
            let p = &mut self.panes[pane.index()];
            p.size_order.checked = Some(signal);
            p.size_order.checked_at = Some(Instant::now());
            if !stale {
                continue;
            }
            let archive = p.archive.as_ref().map(|archive| archive.location.clone());
            let path = p.display_path().to_owned();
            let sent = match archive {
                Some(location) => self.archive_navigate_impl(
                    pane,
                    ArchiveAction::List(location),
                    Reload::SizeResort,
                    send,
                ),
                None => self.navigate_impl(pane, path, None, Reload::SizeResort, send),
            };
            if !sent {
                // A full queue is retried with the next event.
                self.panes[pane.index()].size_order.checked = None;
            }
        }
    }
}
