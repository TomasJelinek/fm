//! Temporary source staging, no-replace restoration and exact empty-stage cleanup.
use super::*;
use midnight_core::{
    PreparedTransferMove, PreparedTransferMoveEntry, TransferMoveCapture,
    TransferMoveCaptureOutcome, TransferRecovery, TransferRevisionPolicy, TransferTarget,
};
use std::{
    ffi::{OsStr, OsString},
    fs::OpenOptions,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    sync::Mutex,
};
use windows_sys::Wdk::Storage::FileSystem::FILE_OPEN;
#[path = "transfer_move_directory_link.rs"]
mod directory_link;
#[path = "transfer_finalize.rs"]
mod finalization;
#[path = "transfer_folder_flush.rs"]
mod folder_flush;
#[path = "transfer_native_move.rs"]
mod native_move;

/// `revisions` is the Move capture's comparison of the source's pinned revision: a Move
/// accepts a moved ChangeTime (VIEW-05); Rename's replacement keeps it exact.
pub(super) fn prepare(
    path: &Path,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    let (source, capture) = prepare_native_with(path, revisions, cancel)?;
    Ok(PreparedTransferMove {
        source: Box::new(source),
        capture,
    })
}

/// The exact Move capture, as Rename's replacement uses it.
#[cfg(test)]
fn prepare_native(
    path: &Path,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    prepare_native_with(path, TransferRevisionPolicy::Exact, cancel)
}

fn prepare_native_with(
    path: &Path,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let (parent_file, parent_path, name) = open_root_location(path, cancel)?;
    prepare_native_child(parent_file, parent_path, name, revisions, cancel)
}

pub(super) fn prepare_with_links(
    path: &Path,
    links: CopyLinkPolicy,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMoveEntry, FsError> {
    let (parent_file, parent_path, name) = open_root_location(path, cancel)?;
    prepare_child_with_links(
        parent_file,
        parent_path,
        name,
        None,
        links,
        revisions,
        cancel,
    )
}

fn open_root_location<'a>(
    path: &'a Path,
    cancel: &Cancellation,
) -> Result<(Arc<File>, PathBuf, &'a OsStr), FsError> {
    checked(cancel, path)?;
    validate_path(path, false)?;
    let parent_path = path
        .parent()
        .ok_or_else(|| invalid(path, "source needs a parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid(path, "source needs a basename"))?;
    // This metadata handle pins the parent's identity (its file ID cannot be reused while
    // open). Traverse access does take part in share-access checks, so while it is held
    // the parent cannot be renamed or removed (nor while the source below it is open,
    // whatever the sharing). Every mutation still works handle-relatively and revalidates
    // the path binding first; a reported path can name the location from that last
    // revalidation. Recovery-only rights and DACL checks are deferred until a
    // cross-volume proof path actually needs staging.
    let parent_file = Arc::new(
        OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent_path)
            .map_err(|e| error(path, e))?,
    );
    Ok((parent_file, parent_path.to_owned(), name))
}

pub(super) fn prepare_child_with_links(
    parent: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    listed: Option<Native>,
    policy: CopyLinkPolicy,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMoveEntry, FsError> {
    let path = parent_path.join(name);
    checked(cancel, &path)?;
    validate_name(name).map_err(|e| error(&path, e))?;
    let original = native::open_relative(
        &parent,
        name,
        FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        0,
    )
    .map_err(|e| error(&path, e))?;
    let native = Native::query(&original).map_err(|e| error(&path, e))?;
    let original_identity = identity(&original, &path)?;
    if listed.is_some_and(|listed| listed.kind() != native.kind()) {
        return Err(invalid(&path, "Move source changed after enumeration"));
    }
    if native.kind() == EntryKind::DirectoryLink {
        if policy == CopyLinkPolicy::FollowDirectoryContents {
            let proof = Arc::new(source_links::OriginalLink::capture(
                &original,
                &path,
                native,
                Some(original_identity),
            )?);
            let capture =
                directory_link::prepare(parent, parent_path, name, proof.clone(), policy, cancel)?;
            return Ok(PreparedTransferMoveEntry::DirectoryLink {
                source: directory_link::deferred_source(original, proof),
                capture,
            });
        }
        let source = source::Source::open_with_links(
            parent.clone(),
            parent_path.clone(),
            name,
            policy,
            cancel,
        )?;
        let proof = source
            .original_link()
            .filter(|proof| proof.entry.identity == original_identity)
            .ok_or_else(|| invalid(&path, "original Move directory link changed"))?;
        let capture = directory_link::prepare(parent, parent_path, name, proof, policy, cancel)?;
        source.revalidate(cancel)?;
        return Ok(PreparedTransferMoveEntry::DirectoryLink {
            source: Box::new(source),
            capture,
        });
    }
    let ordinary = prepare_child_with_policy(parent, parent_path, name, policy, revisions, cancel)?;
    if ordinary.capture.entry().identity != original_identity {
        return Err(invalid(&path, "Move source changed while preparing"));
    }
    Ok(PreparedTransferMoveEntry::Ordinary(ordinary))
}

pub(super) fn prepare_child(
    parent_file: Arc<File>,
    parent_path: std::path::PathBuf,
    name: &OsStr,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    prepare_child_with_policy(
        parent_file,
        parent_path,
        name,
        CopyLinkPolicy::Reject,
        revisions,
        cancel,
    )
}

fn prepare_child_with_policy(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    policy: CopyLinkPolicy,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    let (source, capture) = prepare_native_child_with_policy(
        parent_file,
        parent_path,
        name,
        policy,
        revisions,
        cancel,
    )?;
    Ok(PreparedTransferMove {
        source: Box::new(source),
        capture,
    })
}

fn prepare_native_child(
    parent_file: Arc<File>,
    parent_path: std::path::PathBuf,
    name: &OsStr,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    prepare_native_child_with_policy(
        parent_file,
        parent_path,
        name,
        CopyLinkPolicy::Reject,
        revisions,
        cancel,
    )
}

fn prepare_native_child_with_policy(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    policy: CopyLinkPolicy,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let path = parent_path.join(name);
    let parent = destination::Directory::from_file(
        parent_file.try_clone().map_err(|e| error(&path, e))?,
        parent_path.to_owned(),
    )?;
    let source =
        source::Source::open_for_move(parent_file, parent_path.to_owned(), name, policy, cancel)?
            .with_move_revisions(revisions);
    // The source's own binding pin; its revision policy is the Move's.
    let expected = parent
        .prepare_target_with(name, revisions, cancel)?
        .ok_or_else(|| invalid(&path, "Move source disappeared while preparing"))?;
    if expected.entry().identity != source.entry().identity {
        return Err(invalid(&path, "Move source changed while preparing"));
    }
    let capture = Arc::new(Capture {
        entry: expected.entry().clone(),
        expected,
        file: source.native_file(),
        name: name.to_owned(),
        parent,
        state: Mutex::new(State::default()),
        _namespace_owner: super::namespace::owner(&[source.entry().identity]),
    });
    Ok((source, capture))
}

impl Capture {
    /// The Move source's own binding check. A file keeps its full content revision. A
    /// directory compares identity only: NTFS updates its write/change times whenever a
    /// child changes, and a native rename moves whatever it contains (descendants are
    /// deliberately not enumerated), so such changes must not fail the Move.
    fn revalidate_source(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        if self.entry.item.kind != EntryKind::Directory {
            return self.expected.revalidate(cancel).map_err(|mut failure| {
                // The shared conflict pin speaks of a destination; this is the source.
                if failure.message == "consented destination changed" {
                    failure.message = "Move source changed".into();
                }
                failure
            });
        }
        checked(cancel, path)?;
        self.parent.revalidate(cancel)?;
        let visible = native::open_relative(
            self.parent.native_file(),
            &self.name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            windows_sys::Wdk::Storage::FileSystem::FILE_OPEN,
            0,
        )
        .map_err(|e| error(path, e))?;
        if identity(&visible, path)? != self.entry.identity {
            return Err(invalid(path, "Move source changed"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct State {
    outcome: Option<TransferMoveCaptureOutcome>,
    retained: Option<destination::Directory>,
    /// The empty private stage of a failed capture whose removal failed (see
    /// `keep_or_remove_stage_in`). The next capture reuses it; without one,
    /// `take_stage_cleanup` hands it to the engine's cleanup. A capture that moves the
    /// source consumes it, so it is never set together with `retained`.
    unused: Option<destination::Directory>,
    native_outcome: Option<midnight_core::TransferNativeMoveOutcome>,
}
struct Capture {
    entry: TransferEntry,
    expected: Arc<dyn TransferTarget>,
    file: Arc<File>,
    name: OsString,
    parent: destination::Directory,
    state: Mutex<State>,
    // Last: source File, consent guard, parent and retained recovery state close first.
    _namespace_owner: super::namespace::NativeOwner,
}
impl TransferMoveCapture for Capture {
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(
            Arc<dyn TransferTarget>,
            midnight_core::TransferReplacementPolicy,
        )>,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferNativeMoveOutcome, FsError> {
        native_move::attempt(self, destination, name, replace, cancel)
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferDirectoryRemoval, FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        if self.entry.item.kind != EntryKind::Directory
            || identity(&self.file, path)? != self.entry.identity
            || self
                .parent
                .inspect(&self.name, cancel)?
                .is_none_or(|entry| {
                    entry.identity != self.entry.identity || entry.item.kind != EntryKind::Directory
                })
        {
            return Err(invalid(path, "Move directory identity changed"));
        }
        finalization::qualify_directory(&self.file, path)?;
        checked(cancel, path)?;
        // Exact held DELETE handle; the kernel refuses a nonempty directory.
        super::namespace::change(cancel, |mutation| {
            mutation
                .remove(&self.file, cancel, || native::remove(&self.file))
                .map_err(|e| error(path, e))
        })?;
        Ok(midnight_core::TransferDirectoryRemoval {
            state: midnight_core::TransferSourceRemoval::DeletePending,
            durability_confirmed: false,
        })
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
        self.parent.check_recovery_parent(cancel)?;
        if self.entry.item.kind == EntryKind::Directory {
            finalization::qualify_directory(&self.file, &self.entry.item.path)?;
            return destination::Directory::from_file(
                self.file
                    .try_clone()
                    .map_err(|e| error(&self.entry.item.path, e))?,
                self.entry.item.path.clone(),
            )?
            .check_recovery_parent(cancel);
        }
        finalization::qualify(&self.file, &self.entry.item.path)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferMoveFinalizer>, FsError> {
        finalization::prepare(self, cancel)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate_source(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.capture_with(cancel, || {}, || {})
    }
    fn restore(&self) -> Result<(), FsError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retained.is_none()
            || matches!(
                state.outcome,
                Some(TransferMoveCaptureOutcome::Restored { .. })
            )
        {
            return Ok(());
        }
        let cleanup = Cancellation::default();
        // Putting the user's object back into the directory it came from needs only the
        // parent's identity binding; the DACL trust policy guards staging, and a parent
        // permission change during the operation must not strand the original in the
        // owner-only stage.
        self.parent.revalidate(&cleanup)?;
        // Rename exactly our selected held object. A new original-name winner is
        // never replaced, even if the winner was created after cancellation.
        self.parent
            .capture_handle(&self.file, &self.name, &cleanup)?;
        state.outcome = Some(TransferMoveCaptureOutcome::Restored {
            source: self.entry.item.path.clone(),
            error: invalid(&self.entry.item.path, "temporary source staging restored"),
        });
        Ok(())
    }
    fn restoration_completed(&self) -> bool {
        matches!(
            self.state.lock().unwrap_or_else(|e| e.into_inner()).outcome,
            Some(TransferMoveCaptureOutcome::Restored { .. })
        )
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn midnight_core::TransferMoveStageCleanup>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .retained
            .take()
            .or_else(|| state.unused.take())
            .map(|directory| {
                Box::new(StageCleanup(directory))
                    as Box<dyn midnight_core::TransferMoveStageCleanup>
            })
    }
}
struct StageCleanup(destination::Directory);
impl midnight_core::TransferMoveStageCleanup for StageCleanup {
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        self.0.remove_empty_stage()
    }
}
impl Capture {
    fn capture_with(
        &self,
        cancel: &Cancellation,
        before_capture: impl FnOnce(),
        after_capture: impl FnOnce(),
    ) -> Result<TransferMoveCaptureOutcome, FsError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(outcome) = &state.outcome {
            return Ok(outcome.clone());
        }
        super::namespace::change(cancel, |mutation| {
            self.capture_with_in(&mut state, cancel, before_capture, after_capture, mutation)
        })
    }
    fn capture_with_in(
        &self,
        state: &mut State,
        cancel: &Cancellation,
        before_capture: impl FnOnce(),
        after_capture: impl FnOnce(),
        mutation: &mut super::namespace::Mutation<'_>,
    ) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.revalidate(cancel)?;
        self.parent.check_recovery_parent(cancel)?;
        mutation.prepare_file(&self.file, cancel)?;
        let recovery = match state.unused.take() {
            Some(stage) => stage,
            None => destination::recovery::source_stage_in(&self.parent, cancel, mutation)?,
        };
        if let Err(failure) = self.revalidate(cancel) {
            keep_or_remove_stage_in(state, recovery, mutation);
            return Err(failure);
        }
        before_capture();
        let path = match recovery.capture_handle_in(&self.file, &self.name, cancel, mutation) {
            Ok(path) => path,
            Err(failure) => {
                keep_or_remove_stage_in(state, recovery, mutation);
                return Err(failure);
            }
        };
        after_capture();
        let cleanup = Cancellation::default();
        let inspected = recovery.inspect(&self.name, &cleanup);
        let receipt = TransferRecovery {
            path,
            identity: Some(self.entry.identity),
        };
        let failure = match inspected {
            Ok(Some(entry)) if entry.identity == self.entry.identity => {
                checked(cancel, &self.entry.item.path).err()
            }
            Ok(_) => Some(invalid(
                &self.entry.item.path,
                "captured Move root identity changed; original handle remains retained",
            )),
            Err(error) => Some(error),
        };
        let outcome = if let Some(error) = failure {
            match self
                .parent
                .capture_handle_in(&self.file, &self.name, &cleanup, mutation)
            {
                Ok(_) => TransferMoveCaptureOutcome::Restored {
                    source: self.entry.item.path.clone(),
                    error,
                },
                Err(restore) => {
                    let mut error = error;
                    error.message.push_str(&format!(
                        "; source restoration failed without replacement: {restore}"
                    ));
                    TransferMoveCaptureOutcome::RecoveryRequired {
                        source: self.entry.item.path.clone(),
                        recovery: vec![receipt],
                        error,
                    }
                }
            }
        } else {
            TransferMoveCaptureOutcome::Captured {
                source: self.entry.item.path.clone(),
                recovery: receipt,
            }
        };
        // This directory and the root handle remain pinned until the authority is
        // dropped. No Drop implementation unlinks originals or recovery directories.
        state.retained = Some(recovery);
        state.outcome = Some(outcome.clone());
        Ok(outcome)
    }
}

/// Removes, in the capture's own turn, the empty private stage of a capture that moved
/// nothing. If this turn created the stage, the reservation made at its creation covers
/// the removal, so peers cannot refuse it. A stage reused from an earlier attempt is
/// reserved again in this turn (by the capture's rename, or else by this removal), and
/// granted peers can refuse that before anything moved; the refused turn then allows no
/// further change, so the removal fails. In that case, and when the removal itself
/// fails (another entry in the stage, an I/O error), the capture keeps the stage instead
/// of dropping it unreported: its next attempt reuses it, and otherwise
/// `take_stage_cleanup` returns it for the engine to remove or report.
fn keep_or_remove_stage_in(
    state: &mut State,
    stage: destination::Directory,
    mutation: &mut super::namespace::Mutation<'_>,
) {
    // The removal consumes a clone, so a failure leaves this handle and binding intact.
    if stage.clone().remove_empty_stage_in(mutation).is_err() {
        state.unused = Some(stage);
    }
}

#[cfg(test)]
#[path = "transfer_move_tests.rs"]
mod tests;
