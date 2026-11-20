//! Folders that transfers hold open inside one another, bounded for the whole process.
//!
//! Every folder level of a Copy or Move keeps descriptors open while its contents are
//! transferred: the source folder, its entry stream and the destination folder. Ordinary
//! folders may nest arbitrarily deep, so without a bound a deep tree would exhaust the
//! process's open-file limit (EMFILE) and make everything else that opens a file meanwhile
//! fail: other jobs, folder listings, watches, viewers. Levels are therefore counted
//! across every transfer (and their metadata discovery) of the process, and a level past
//! a share of that limit is refused with a clear error, long before the limit itself.
//! The application raises its soft limit at start (`open_files`), before the first
//! transfer sizes the levels: to the hard limit, at most 65,536, which is 4,096 levels.
use super::*;
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

/// Descriptors one folder level holds: the source folder, its entry stream while its
/// names are read, and the destination folder.
const DESCRIPTORS_PER_LEVEL: u64 = 3;
/// Folder levels take at most a quarter of the open-file limit; the rest stays for
/// everything else the application opens.
const SHARE: u64 = 4;
/// Room for ordinary trees even under a small limit.
const MIN_LEVELS: usize = 32;
/// No pathological nesting under a huge limit.
const MAX_LEVELS: usize = 4096;

/// A count of folder levels held open, with its limit.
pub(super) struct Levels {
    held: AtomicUsize,
    limit: usize,
}

/// One folder level held open; dropping it releases the level.
pub(super) struct Level(&'static Levels);

impl Drop for Level {
    fn drop(&mut self) {
        self.0.held.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Levels {
    pub(super) const fn new(limit: usize) -> Self {
        Self {
            held: AtomicUsize::new(0),
            limit,
        }
    }

    /// Hold the folder `path` open as one more level, or refuse it clearly when every
    /// level is in use.
    pub(super) fn enter(&'static self, path: &Path) -> Result<Level, FsError> {
        self.held
            .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < self.limit).then_some(held + 1)
            })
            .map(|_| Level(self))
            .map_err(|_| {
                unsupported(
                    path,
                    &format!(
                        "Transfers keep each folder open while they work inside it, and {} \
                         folders open inside one another is the most this process allows \
                         within its open-file limit (which it raises when it starts, to the \
                         system's hard limit but at most 65,536); this folder lies deeper \
                         (transfers that run at the same time count too)",
                        self.limit
                    ),
                )
            })
    }

    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.held.load(Ordering::Acquire)
    }
}

/// The process's levels, sized from its open-file limit when first used.
pub(super) fn levels() -> &'static Levels {
    #[cfg(test)]
    if let Some(levels) = OVERRIDE.with(std::cell::Cell::get) {
        return levels;
    }
    static LEVELS: OnceLock<Levels> = OnceLock::new();
    LEVELS.get_or_init(|| {
        Levels::new(limit_for(
            rustix::process::getrlimit(rustix::process::Resource::Nofile).current,
        ))
    })
}

/// The levels for an open-file limit of `descriptors` (`None`: unlimited).
fn limit_for(descriptors: Option<u64>) -> usize {
    descriptors
        .map_or(MAX_LEVELS, |descriptors| {
            usize::try_from(descriptors / SHARE / DESCRIPTORS_PER_LEVEL).unwrap_or(MAX_LEVELS)
        })
        .clamp(MIN_LEVELS, MAX_LEVELS)
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<&'static Levels>> = const { std::cell::Cell::new(None) };
}

/// Tests: `levels` on this thread are `levels` until the returned guard drops, so that
/// a test's own budget neither depends on nor disturbs the tests that run beside it.
#[cfg(test)]
pub(super) fn scoped(levels: &'static Levels) -> Scoped {
    Scoped(OVERRIDE.with(|cell| cell.replace(Some(levels))))
}

#[cfg(test)]
pub(super) struct Scoped(Option<&'static Levels>);

#[cfg(test)]
impl Drop for Scoped {
    fn drop(&mut self) {
        OVERRIDE.with(|cell| cell.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_is_a_quarter_of_the_open_file_limit_within_its_bounds() {
        // The common default soft limit: 1024 descriptors.
        assert_eq!(limit_for(Some(1024)), 85);
        assert_eq!(limit_for(Some(4096)), 341);
        assert_eq!(limit_for(Some(65_536)), MAX_LEVELS);
        assert_eq!(limit_for(Some(64)), MIN_LEVELS);
        assert_eq!(limit_for(Some(1 << 20)), MAX_LEVELS);
        assert_eq!(limit_for(None), MAX_LEVELS);
        // Whatever the limit, the levels' descriptors stay well inside it.
        for descriptors in [256, 1024, 4096, 65_536, 1 << 20] {
            let levels = limit_for(Some(descriptors)) as u64;
            assert!(
                levels * DESCRIPTORS_PER_LEVEL <= descriptors / 2,
                "{descriptors}"
            );
        }
    }

    #[test]
    fn levels_are_refused_clearly_once_all_are_held_and_return_when_released() {
        static LEVELS: Levels = Levels::new(2);
        let path = Path::new("/deep/folder");
        let first = LEVELS.enter(path).unwrap();
        let second = LEVELS.enter(path).unwrap();
        let Err(refused) = LEVELS.enter(path) else {
            panic!("a third level past a limit of two");
        };
        assert_eq!(refused.kind, io::ErrorKind::Unsupported);
        assert!(
            refused
                .message
                .contains("2 folders open inside one another"),
            "{}",
            refused.message
        );
        assert_eq!(LEVELS.held(), 2);
        drop(first);
        let third = LEVELS.enter(path).unwrap();
        drop((second, third));
        assert_eq!(LEVELS.held(), 0);
    }

    /// The engine keeps every folder it works inside open, as the frames here do. A tree
    /// deeper than the levels is refused clearly at the first folder past them, and every
    /// level returns once the transfer lets go, so a deep tree never comes near the
    /// process's open-file limit.
    #[test]
    fn a_deep_tree_is_refused_clearly_past_the_levels_and_releases_them() {
        static LEVELS: Levels = Levels::new(40);
        let _levels = scoped(&LEVELS);
        let root = tempfile::tempdir().unwrap();
        let tree = root.path().join("deep");
        let mut bottom = tree.clone();
        for _ in 0..60 {
            bottom.push("d");
        }
        std::fs::create_dir_all(&bottom).unwrap();
        let cancel = Cancellation::default();
        for policy in [CopyLinkPolicy::Preserve, CopyLinkPolicy::FollowLinks] {
            let mut frames = vec![
                crate::LinuxFileSystem::new(None)
                    .prepare_transfer_source_with_links(&tree, policy, &cancel)
                    .unwrap(),
            ];
            let refused = loop {
                match frames.last_mut().unwrap().next_child(&cancel) {
                    Ok(Some(child)) => frames.push(child),
                    Ok(None) => panic!("{policy:?}: the tree ended before the levels did"),
                    Err(error) => break error,
                }
            };
            // The selected root is not a level: 40 folders below it are.
            assert_eq!(frames.len(), 41, "{policy:?}");
            assert_eq!(LEVELS.held(), 40);
            assert_eq!(refused.kind, io::ErrorKind::Unsupported);
            assert!(
                refused
                    .message
                    .contains("40 folders open inside one another"),
                "{}",
                refused.message
            );
            // Discovery draws on the same levels, and only loses its totals.
            let discovery = frames[0].discover(&cancel, &mut |_| {}).unwrap_err();
            assert!(
                discovery.message.contains("open inside one another"),
                "{}",
                discovery.message
            );
            drop(frames);
            assert_eq!(LEVELS.held(), 0, "{policy:?}: every level is released");
        }
    }
}
