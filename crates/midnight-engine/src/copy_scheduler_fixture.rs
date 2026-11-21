use super::*;
use std::{
    collections::HashSet,
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
    ops::{Deref, DerefMut},
    sync::Condvar,
    time::{Instant, SystemTime},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Point {
    ReadObserved,
    Read,
    Written,
    MoveAttempt,
    Moved,
    Cleanup,
    Dropped,
}

#[derive(Default)]
struct Calls {
    armed: HashSet<(usize, Point)>,
    entered: HashSet<(usize, Point)>,
    released: HashSet<(usize, Point)>,
    release_all: bool,
    timed_out: bool,
    order: Vec<(usize, Point)>,
}

pub(super) struct Control {
    sources: [PathBuf; 3],
    markers: [u8; 3],
    opaque: bool,
    fail_write: Option<usize>,
    calls: Mutex<Calls>,
    changed: Condvar,
}
impl Control {
    pub(super) fn arm(&self, index: usize, point: Point) {
        self.calls.lock().unwrap().armed.insert((index, point));
    }
    fn source_index(&self, path: &Path) -> usize {
        self.sources
            .iter()
            .position(|p| p == path)
            .expect("unknown native source path")
    }
    fn write_index(&self, data: &[u8]) -> usize {
        let index = self
            .markers
            .iter()
            .position(|marker| *marker == data[0])
            .expect("unknown native byte tag");
        assert!(data.iter().all(|byte| *byte == self.markers[index]));
        index
    }
    fn hold(&self, index: usize, point: Point) {
        let key = (index, point);
        let mut calls = self.calls.lock().unwrap();
        if !calls.entered.insert(key) {
            return;
        }
        calls.order.push(key);
        // Held points follow successful delegated native operations. MoveAttempt
        // is an unheld negative witness at entry; Moved proves actual commit.
        // Cleanup retains the stage until the actual destructor can run.
        let deadline = Instant::now() + Duration::from_secs(30);
        while calls.armed.contains(&key) && !calls.released.contains(&key) && !calls.release_all {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                calls.timed_out = true;
                break;
            }
            calls = self.changed.wait_timeout(calls, remaining).unwrap().0;
        }
    }
    pub(super) fn release(&self, index: usize, point: Point) {
        self.calls.lock().unwrap().released.insert((index, point));
        self.changed.notify_all();
    }
    pub(super) fn release_all(&self) {
        self.calls.lock().unwrap().release_all = true;
        self.changed.notify_all();
    }
    pub(super) fn timed_out(&self) -> bool {
        self.calls.lock().unwrap().timed_out
    }
    pub(super) fn before(&self, first: (usize, Point), second: (usize, Point)) -> bool {
        let calls = self.calls.lock().unwrap();
        match (
            calls.order.iter().position(|point| *point == first),
            calls.order.iter().position(|point| *point == second),
        ) {
            (Some(first), Some(second)) => first < second,
            _ => false,
        }
    }
    pub(super) async fn wait(&self, index: usize, point: Point, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                if self.calls.lock().unwrap().entered.contains(&(index, point)) {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or(false)
    }
}

pub(super) struct Release(pub(super) Arc<Control>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release_all();
    }
}

pub(super) struct Case {
    _temp: tempfile::TempDir,
    pub(super) sources: [PathBuf; 3],
    pub(super) destination: PathBuf,
    pub(super) control: Arc<Control>,
    contents: [Vec<u8>; 3],
}
impl Case {
    pub(super) fn new(names: [&str; 3], opaque: bool, fail_write: Option<usize>) -> Self {
        // Native-created protected parent is required for actual Move authority.
        let (temp, parent) = super::super::moves::protected_fixture();
        let source = parent.join("source");
        let destination = parent.join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        let sources = names.map(|name| source.join(name));
        let markers = [0x31, 0x53, 0x75];
        let contents = std::array::from_fn(|index| vec![markers[index]; 768 * 1024 + 13 + index]);
        for index in 0..3 {
            fs::write(&sources[index], &contents[index]).unwrap();
        }
        // Positive native identities also make the Linux a/A fixture fail on a
        // case-insensitive mount rather than silently testing one replaced file.
        let provider = midnight_platform::LocalFileSystem::new(None);
        let identities: Vec<_> = sources
            .iter()
            .map(|path| {
                provider
                    .prepare_transfer_source(path, &Cancellation::default())
                    .unwrap()
                    .entry()
                    .identity
            })
            .collect();
        assert_ne!(identities[0], identities[1]);
        assert_ne!(identities[1], identities[2]);
        let control = Arc::new(Control {
            sources: sources.clone(),
            markers,
            opaque,
            fail_write,
            calls: Mutex::default(),
            changed: Condvar::new(),
        });
        Self {
            _temp: temp,
            sources,
            destination,
            control,
            contents,
        }
    }
    pub(super) fn service(&self) -> Arc<Service> {
        Arc::new(Service {
            inner: midnight_platform::LocalFileSystem::new(None),
            control: self.control.clone(),
        })
    }
    pub(super) fn bytes(&self, index: usize) -> &[u8] {
        &self.contents[index]
    }
    pub(super) fn output(&self, index: usize) -> PathBuf {
        self.destination
            .join(self.sources[index].file_name().unwrap())
    }
    pub(super) fn request(&self, index: usize, id: u64, wait: bool) -> CopyRequest {
        let mut result = super::super::request(
            id,
            std::slice::from_ref(&self.sources[index]),
            &self.destination,
        );
        if wait {
            result.start_policy = TransferStartPolicy::WaitForEarlier;
        }
        result
    }
    pub(super) async fn enqueue(
        &self,
        h: &Harness,
        index: usize,
        id: u64,
        moving: bool,
        wait: bool,
    ) {
        let request = self.request(index, id, wait);
        if moving {
            h.controller.enqueue_move(request).await;
        } else {
            h.controller.enqueue(request).await;
        }
    }
}

struct Source {
    inner: Box<dyn TransferSource>,
    reads: usize,
    control: Arc<Control>,
}

impl Source {
    fn wrap(inner: Box<dyn TransferSource>, control: &Arc<Control>) -> Box<dyn TransferSource> {
        Box::new(Self {
            inner,
            reads: 0,
            control: control.clone(),
        })
    }
}

impl TransferSource for Source {
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.inner
            .next_move_child(cancel)
            .map(|next| next.map(|next| wrap_move(next, &self.control)))
    }
    fn next_move_child_or_failure(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<TransferMoveChild>, FsError> {
        if links != CopyLinkPolicy::Reject {
            // As `next_move_child_with_links`, which this fixture leaves unsupported.
            return self
                .next_move_child_with_links(links, cancel)
                .map(|next| next.map(Ok));
        }
        self.inner
            .next_move_child_or_failure(links, cancel)
            .map(|next| next.map(|next| next.map(|entry| wrap_move_entry(entry, &self.control))))
    }
    fn prepare_move_child(
        &self,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMoveEntry, FsError> {
        self.inner
            .prepare_move_child(name, links, cancel)
            .map(|entry| wrap_move_entry(entry, &self.control))
    }
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        self.inner.link_representation(cancel)
    }
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        if self.control.opaque {
            return Ok(None);
        }
        self.inner.native_namespace_scope(cancel)
    }
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.inner.native_identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn open_for_reading(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.open_for_reading(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.inner.discover(cancel, visit)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        let count = self.inner.read_chunk(buffer, cancel)?;
        if count != 0 {
            let index = self.control.source_index(&self.inner.entry().item.path);
            assert!(
                buffer[..count]
                    .iter()
                    .all(|byte| *byte == self.control.markers[index])
            );
            self.control.hold(index, Point::ReadObserved);
            self.reads += 1;
            if self.reads == 2 {
                self.control.hold(index, Point::Read);
            }
        }
        Ok(count)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner
            .next_child(cancel)
            .map(|child| child.map(|child| Self::wrap(child, &self.control)))
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner
            .open_child(name, cancel)
            .map(|child| Self::wrap(child, &self.control))
    }
    fn supports_read_retry(&self) -> bool {
        self.inner.supports_read_retry()
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.retry_read_from(offset, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.inner.seek_for_validation(offset, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.inner.prepare_modified_time(cancel)
    }
}

pub(super) struct Service {
    inner: midnight_platform::LocalFileSystem,
    control: Arc<Control>,
}

// This fixture delegates native owners without background disposal. All holds
// return synchronously, including the explicit actual-stage Drop below. Opaque
// geometry changes conflict admission only; it does not change teardown audit.
impl midnight_core::AuditedSynchronousTransferService for Service {}

impl TransferService for Service {
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner
            .prepare_transfer_source_with_links(path, links, cancel)
            .map(|source| Source::wrap(source, &self.control))
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .prepare_move_destination(path, cancel)
            .map(|directory| Directory::wrap(directory, &self.control))
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        self.inner
            .prepare_move_source(path, cancel)
            .map(|prepared| wrap_move(prepared, &self.control))
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .prepare_checksum_directory(path, cancel)
            .map(|directory| Directory::wrap(directory, &self.control))
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner
            .prepare_transfer_source(path, cancel)
            .map(|source| Source::wrap(source, &self.control))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .prepare_transfer_destination(path, cancel)
            .map(|directory| Directory::wrap(directory, &self.control))
    }
}

fn wrap_move_entry(
    entry: PreparedTransferMoveEntry,
    control: &Arc<Control>,
) -> PreparedTransferMoveEntry {
    match entry {
        PreparedTransferMoveEntry::Ordinary(prepared) => {
            PreparedTransferMoveEntry::Ordinary(wrap_move(prepared, control))
        }
        link => link,
    }
}
fn wrap_move(prepared: PreparedTransferMove, control: &Arc<Control>) -> PreparedTransferMove {
    PreparedTransferMove {
        source: Source::wrap(prepared.source, control),
        capture: Arc::new(Capture {
            inner: prepared.capture,
            control: control.clone(),
        }),
    }
}
struct Capture {
    inner: Arc<dyn TransferMoveCapture>,
    control: Arc<Control>,
}
impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.preflight_finalization(cancel)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.inner.prepare_finalization(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.inner.capture(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.inner.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.inner.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.inner.take_stage_cleanup()
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        self.inner.remove_empty_directory(cancel)
    }
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        let index = self.control.source_index(&self.entry().item.path);
        self.control.hold(index, Point::MoveAttempt);
        let result = self
            .inner
            .try_native_move(destination, name, replace, cancel)?;
        if matches!(result, TransferNativeMoveOutcome::Moved { .. }) {
            self.control.hold(index, Point::Moved);
        }
        Ok(result)
    }
}

struct Directory {
    inner: Arc<dyn TransferDirectory>,
    control: Arc<Control>,
}

impl Directory {
    fn wrap(
        inner: Arc<dyn TransferDirectory>,
        control: &Arc<Control>,
    ) -> Arc<dyn TransferDirectory> {
        Arc::new(Self {
            inner,
            control: control.clone(),
        })
    }
    fn stage(&self, inner: Box<dyn StagedTransferFile>) -> Box<dyn StagedTransferFile> {
        Box::new(Stage {
            inner: NativeStage(Some(inner)),
            index: None,
            control: self.control.clone(),
        })
    }
}

impl TransferDirectory for Directory {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        if self.control.opaque {
            return Ok(None);
        }
        self.inner.native_child_namespace_scope(name, kind, cancel)
    }
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.inner.capabilities()
    }
    fn supports_native_move_replacement(&self, source: TransferIdentity) -> bool {
        self.inner.supports_native_move_replacement(source)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.inner.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .create_directory(name, cancel)
            .map(|directory| Self::wrap(directory, &self.control))
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .open_merge_directory(name, expected, cancel)
            .map(|directory| Self::wrap(directory, &self.control))
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.open_read_child(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner.stage_file(cancel).map(|stage| self.stage(stage))
    }
    fn stage_file_with_metadata(
        &self,
        policy: TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_with_metadata(policy, cancel)
            .map(|stage| self.stage(stage))
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_for_target(name, target, policy, cancel)
            .map(|stage| self.stage(stage))
    }
    fn stage_file_preserving_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_preserving_target(name, target, policy, cancel)
            .map(|stage| self.stage(stage))
    }
    fn admit_link(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.admit_link(representation, cancel)
    }
    fn stage_link(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        self.inner.stage_link(representation, cancel)
    }
}

struct Stage {
    inner: NativeStage,
    index: Option<usize>,
    control: Arc<Control>,
}

struct NativeStage(Option<Box<dyn StagedTransferFile>>);
impl Deref for NativeStage {
    type Target = dyn StagedTransferFile;
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().unwrap()
    }
}
impl DerefMut for NativeStage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_deref_mut().unwrap()
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        drop(self.inner.0.take());
        if let Some(index) = self.index {
            self.control.hold(index, Point::Dropped);
        }
    }
}

impl Read for Stage {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}
impl Write for Stage {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(data)?;
        if count != 0 {
            let index = self.control.write_index(&data[..count]);
            self.index = Some(index);
            self.control.hold(index, Point::Written);
            if self.control.fail_write == Some(index) {
                return Err(io::Error::other(
                    "injected failure after successful native stage write",
                ));
            }
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.sync(cancel)
    }
    fn len(&self) -> Result<u64, FsError> {
        self.inner.len()
    }
    fn set_modified_time(
        &mut self,
        expected: SystemTime,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.set_modified_time(expected, cancel)
    }
    fn modified_time(&self, cancel: &Cancellation) -> Result<SystemTime, FsError> {
        self.inner.modified_time(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        self.inner.revalidate_publication(cancel)
    }
    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.prepare_replacement(name, target, policy, cancel)
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_no_replace(name, cancel)
    }
    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_replace(name, cancel)
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.inner.cleanup()?;
        if let Some(index) = self.index {
            self.control.hold(index, Point::Cleanup);
        }
        Ok(())
    }
}
