//! Link proofs exercise the existing worker without filesystem or referent access.
use super::*;
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsStr,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        Weak,
        atomic::{AtomicBool, AtomicUsize},
        mpsc as blocking,
    },
};
use tokio::sync::Notify;

fn identity(value: u8) -> TransferIdentity {
    TransferIdentity {
        device: 7,
        object: [value; 16],
    }
}

fn entry(path: PathBuf, kind: EntryKind, value: u8) -> TransferEntry {
    TransferEntry {
        item: MutationItem {
            path,
            kind,
            // A link's reported length must never become referent content credit.
            metadata: EntryMetadata {
                size: Some(9_999),
                ..EntryMetadata::default()
            },
        },
        identity: identity(value),
    }
}

fn relative_representation() -> TransferLinkRepresentation {
    TransferLinkRepresentation::linux_symbolic_link(b"../missing/\xff").unwrap()
}

fn active(path: &Path, cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error(
            path,
            io::ErrorKind::Interrupted,
            "Fixture call cancelled.",
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone)]
struct Node {
    entry: TransferEntry,
    representation: Option<TransferLinkRepresentation>,
    children: Vec<Node>,
}

impl Node {
    fn link(path: PathBuf, representation: TransferLinkRepresentation, value: u8) -> Self {
        let kind = match representation.kind() {
            TransferLinkKind::WindowsDirectorySymbolicLink | TransferLinkKind::WindowsJunction => {
                EntryKind::DirectoryLink
            }
            _ => EntryKind::Symlink,
        };
        Self {
            entry: entry(path, kind, value),
            representation: Some(representation),
            children: vec![],
        }
    }
}

#[derive(Clone)]
struct Object {
    entry: TransferEntry,
    representation: Option<TransferLinkRepresentation>,
}

#[derive(Default)]
struct Namespace {
    stages: BTreeMap<u8, TransferLinkRepresentation>,
    published: BTreeMap<PathBuf, Object>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

pub(super) struct SyncBlock {
    pub(super) entered: Arc<Notify>,
    pub(super) release: blocking::Receiver<()>,
}

struct ObservedResources {
    io: Arc<Semaphore>,
    mutation: Arc<Semaphore>,
    capacity: Arc<Semaphore>,
}

#[derive(Default)]
pub(super) struct Control {
    namespace: Mutex<Namespace>,
    events: Mutex<Vec<&'static str>>,
    drops: Mutex<Vec<(&'static str, bool)>>,
    drop_threads: Mutex<Vec<std::thread::ThreadId>>,
    resources: Mutex<Option<ObservedResources>>,
    resource_snapshots: Mutex<Vec<(&'static str, usize, usize, usize)>>,
    policies: Mutex<Vec<CopyLinkPolicy>>,
    registration: Mutex<Weak<Registration>>,
    pub(super) source_opens: AtomicUsize,
    pub(super) destination_opens: AtomicUsize,
    legacy_opens: AtomicUsize,
    source_queries: AtomicUsize,
    stage_queries: AtomicUsize,
    pub(super) stages_created: AtomicUsize,
    pub(super) cleanups: AtomicUsize,
    publishes: AtomicUsize,
    referent_reads: AtomicUsize,
    regular_payload: Mutex<Option<Vec<u8>>>,
    regular_reads: AtomicUsize,
    file_readbacks: AtomicUsize,
    file_stages: AtomicUsize,
    child_reads: AtomicUsize,
    fail_second_open: AtomicBool,
    replacement_on_open: Mutex<Option<TransferLinkRepresentation>>,
    refuse_admission: AtomicBool,
    pub(super) fail_stage_creation: AtomicBool,
    mismatch_readback: AtomicBool,
    change_binding_after_readback: AtomicBool,
    invalid_binding: AtomicBool,
    change_payload_after_readback: AtomicBool,
    changed_payload: AtomicBool,
    substitute_stage: AtomicBool,
    substitute_publication: AtomicBool,
    pub(super) refuse_cleanup: AtomicBool,
    cleanup_unconfirmed: AtomicBool,
    pub(super) late_collision: AtomicBool,
    cancel_after_publish: AtomicBool,
    publication_outcome: AtomicUsize,
    pub(super) sync_block: Mutex<Option<SyncBlock>>,
    pub(super) cleanup_block: Mutex<Option<SyncBlock>>,
}

impl Control {
    fn event(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }

    fn record_drop(&self, owner: &'static str) {
        self.record_resources(owner);
        let registered = self.registration.lock().unwrap().upgrade().is_some();
        self.drops.lock().unwrap().push((owner, registered));
        self.drop_threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }

    fn record_resources(&self, owner: &'static str) {
        if let Some(resources) = self.resources.lock().unwrap().as_ref() {
            self.resource_snapshots.lock().unwrap().push((
                owner,
                resources.io.available_permits(),
                resources.mutation.available_permits(),
                resources.capacity.available_permits(),
            ));
        }
    }

    pub(super) fn observe_resources(
        &self,
        io: Arc<Semaphore>,
        mutation: Arc<Semaphore>,
        capacity: Arc<Semaphore>,
    ) {
        *self.resources.lock().unwrap() = Some(ObservedResources {
            io,
            mutation,
            capacity,
        });
    }

    pub(super) fn resource_snapshots(&self) -> Vec<(&'static str, usize, usize, usize)> {
        self.resource_snapshots.lock().unwrap().clone()
    }

    pub(super) fn drop_threads(&self) -> Vec<std::thread::ThreadId> {
        self.drop_threads.lock().unwrap().clone()
    }

    pub(super) fn owner_drops(&self) -> Vec<(&'static str, bool)> {
        self.drops.lock().unwrap().clone()
    }

    pub(super) fn staged_count(&self) -> usize {
        self.namespace.lock().unwrap().stages.len()
    }

    fn output(&self, path: &Path) -> Option<TransferLinkRepresentation> {
        self.namespace
            .lock()
            .unwrap()
            .published
            .get(path)
            .and_then(|object| object.representation.clone())
    }
}

pub(super) struct Registration {
    control: Weak<Control>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(control) = self.control.upgrade() {
            control.record_resources("provider");
        }
    }
}

pub(super) struct Service {
    roots: Vec<Node>,
    control: Arc<Control>,
}

impl TransferService for Service {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            control: self.control.clone(),
        }))
    }

    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.control.legacy_opens.fetch_add(1, Ordering::SeqCst);
        self.source(path, cancel)
    }

    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.control.policies.lock().unwrap().push(links);
        self.source(path, cancel)
    }

    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.control
            .destination_opens
            .fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            control: self.control.clone(),
        }))
    }
}

impl Service {
    fn source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        active(path, cancel)?;
        self.control.source_opens.fetch_add(1, Ordering::SeqCst);
        if path.file_name() == Some(OsStr::new("second"))
            && self.control.fail_second_open.swap(false, Ordering::SeqCst)
        {
            return Err(error(
                path,
                io::ErrorKind::NotConnected,
                "Source unavailable.",
            ));
        }
        let mut node = self
            .roots
            .iter()
            .find(|node| node.entry.item.path == path)
            .expect("fixture root")
            .clone();
        if let Some(replacement) = self.control.replacement_on_open.lock().unwrap().clone() {
            node.representation = Some(replacement);
        }
        let registration = self.control.registration.lock().unwrap().upgrade();
        Ok(Box::new(Source::new(
            node,
            self.control.clone(),
            registration,
        )))
    }
}

struct Source {
    node: Node,
    children: VecDeque<Node>,
    control: Arc<Control>,
    _registration: Option<Arc<Registration>>,
    offset: usize,
}

impl Source {
    fn new(node: Node, control: Arc<Control>, registration: Option<Arc<Registration>>) -> Self {
        Self {
            children: node.children.clone().into(),
            node,
            control,
            _registration: registration,
            offset: 0,
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.control.record_drop("source");
    }
}

impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.node.entry
    }

    fn native_identity(&self) -> Option<TransferIdentity> {
        Some(self.node.entry.identity)
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        active(&self.node.entry.item.path, cancel)
    }

    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        self.revalidate(cancel)?;
        self.control.event("source representation");
        self.control.source_queries.fetch_add(1, Ordering::SeqCst);
        if self.control.invalid_binding.load(Ordering::SeqCst) {
            return Err(error(
                &self.node.entry.item.path,
                io::ErrorKind::InvalidData,
                "The original name no longer denotes the retained link.",
            ));
        }
        if self.control.changed_payload.load(Ordering::SeqCst) {
            return Ok(TransferLinkRepresentation::linux_symbolic_link(b"changed").unwrap());
        }
        self.node.representation.clone().ok_or_else(|| {
            error(
                &self.node.entry.item.path,
                io::ErrorKind::Unsupported,
                "This object has no supported link representation.",
            )
        })
    }

    fn read_chunk(&mut self, bytes: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if self.node.entry.item.kind == EntryKind::File {
            active(&self.node.entry.item.path, cancel)?;
            let payload = self.control.regular_payload.lock().unwrap();
            let payload = payload.as_ref().expect("ordinary-file fixture payload");
            let count = bytes.len().min(payload.len() - self.offset);
            bytes[..count].copy_from_slice(&payload[self.offset..self.offset + count]);
            self.offset += count;
            self.control
                .regular_reads
                .fetch_add(count, Ordering::SeqCst);
            return Ok(count);
        }
        self.control.referent_reads.fetch_add(1, Ordering::SeqCst);
        panic!("link Copy must never read a referent")
    }

    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.control.child_reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(self.node.entry.item.kind, EntryKind::Directory);
        Ok(self.children.pop_front().map(|node| {
            Box::new(Self::new(
                node,
                self.control.clone(),
                self._registration.clone(),
            )) as Box<dyn TransferSource>
        }))
    }
}

struct Directory {
    path: PathBuf,
    control: Arc<Control>,
}

impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }

    fn identity(&self) -> TransferIdentity {
        identity(50)
    }

    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities {
            directory_merge: true,
            conditional_overwrite: true,
            recoverable_replacement: true,
        }
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        active(&self.path, cancel)
    }

    fn inspect(&self, name: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        Ok(self
            .control
            .namespace
            .lock()
            .unwrap()
            .published
            .get(&self.path.join(name))
            .map(|object| object.entry.clone()))
    }

    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        Ok(self.inspect(name, cancel)?.map(|entry| {
            Arc::new(Target {
                entry,
                control: self.control.clone(),
            }) as Arc<dyn TransferTarget>
        }))
    }

    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }

    fn create_directory(
        &self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let path = self.path.join(name);
        self.control.namespace.lock().unwrap().published.insert(
            path.clone(),
            Object {
                entry: entry(path.clone(), EntryKind::Directory, 51),
                representation: None,
            },
        );
        Ok(Arc::new(Self {
            path,
            control: self.control.clone(),
        }))
    }

    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        assert!(
            self.control.regular_payload.lock().unwrap().is_some(),
            "link Copy must use its distinct stage capability"
        );
        self.control.file_stages.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FileStage {
            data: Cursor::new(vec![]),
            destination: self.path.clone(),
            control: self.control.clone(),
            published: false,
        }))
    }

    fn admit_link(&self, _: &TransferLinkRepresentation, _: &Cancellation) -> Result<(), FsError> {
        if self.control.refuse_admission.load(Ordering::SeqCst) {
            Err(error(
                &self.path,
                io::ErrorKind::Unsupported,
                "This endpoint does not admit the native link kind.",
            ))
        } else {
            Ok(())
        }
    }

    fn stage_link(
        &self,
        representation: &TransferLinkRepresentation,
        _: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        let number = self.control.stages_created.fetch_add(1, Ordering::SeqCst);
        let value = u8::try_from(number + 100).unwrap();
        self.control.event("stage");
        self.control
            .namespace
            .lock()
            .unwrap()
            .stages
            .insert(value, representation.clone());
        let stage: Box<dyn StagedTransferLink> = Box::new(Stage {
            value,
            path: self.path.join(format!(".owned-link-{value}")),
            destination: self.path.clone(),
            control: self.control.clone(),
            published: None,
        });
        if self.control.fail_stage_creation.load(Ordering::SeqCst) {
            Err(TransferLinkStageError {
                error: error(
                    &self.path,
                    io::ErrorKind::PermissionDenied,
                    "Setting the created link failed.",
                ),
                stage: Some(stage),
            })
        } else {
            Ok(stage)
        }
    }
}

struct FileStage {
    data: Cursor<Vec<u8>>,
    destination: PathBuf,
    control: Arc<Control>,
    published: bool,
}

impl Read for FileStage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.data.read(bytes)?;
        self.control
            .file_readbacks
            .fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }
}

impl Write for FileStage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.data.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.data.flush()
    }
}

impl Seek for FileStage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.data.seek(position)
    }
}

impl StagedTransferFile for FileStage {
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }

    fn len(&self) -> Result<u64, FsError> {
        Ok(self.data.get_ref().len() as u64)
    }

    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        let path = self.destination.join(name);
        let mut namespace = self.control.namespace.lock().unwrap();
        assert!(!namespace.published.contains_key(&path));
        let mut entry = entry(path.clone(), EntryKind::File, 99);
        entry.item.metadata.size = Some(self.data.get_ref().len() as u64);
        namespace.published.insert(
            path.clone(),
            Object {
                entry,
                representation: None,
            },
        );
        namespace
            .files
            .insert(path.clone(), self.data.get_ref().clone());
        self.published = true;
        Ok(TransferPublication::Published(path))
    }

    fn cleanup(&mut self) -> Result<(), FsError> {
        if !self.published {
            self.data.get_mut().clear();
        }
        Ok(())
    }
}

struct Target {
    entry: TransferEntry,
    control: Arc<Control>,
}

impl TransferTarget for Target {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        let namespace = self.control.namespace.lock().unwrap();
        if namespace
            .published
            .get(&self.entry.item.path)
            .is_some_and(|object| object.entry.identity == self.entry.identity)
        {
            Ok(())
        } else {
            Err(error(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "The conflict winner changed.",
            ))
        }
    }
}

struct Stage {
    value: u8,
    path: PathBuf,
    destination: PathBuf,
    control: Arc<Control>,
    published: Option<PathBuf>,
}

impl Drop for Stage {
    fn drop(&mut self) {
        self.control.record_drop("stage");
    }
}

impl StagedTransferLink for Stage {
    fn link_representation(&self, _: &Cancellation) -> Result<TransferLinkRepresentation, FsError> {
        self.control.event("stage readback");
        self.control.stage_queries.fetch_add(1, Ordering::SeqCst);
        if self
            .control
            .change_binding_after_readback
            .load(Ordering::SeqCst)
        {
            self.control.invalid_binding.store(true, Ordering::SeqCst);
        }
        if self
            .control
            .change_payload_after_readback
            .load(Ordering::SeqCst)
        {
            self.control.changed_payload.store(true, Ordering::SeqCst);
        }
        if self.control.mismatch_readback.load(Ordering::SeqCst) {
            return Ok(
                TransferLinkRepresentation::linux_symbolic_link(b"wrong raw target").unwrap(),
            );
        }
        Ok(self.control.namespace.lock().unwrap().stages[&self.value].clone())
    }

    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.control.event("sync");
        let block = self.control.sync_block.lock().unwrap().take();
        if let Some(block) = block {
            block.entered.notify_one();
            block.release.recv_timeout(Duration::from_secs(4)).unwrap();
        }
        active(&self.path, cancel)
    }

    fn revalidate_stage(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        self.control.event("stage binding");
        active(&self.path, cancel)?;
        if self.control.substitute_stage.load(Ordering::SeqCst) {
            return Err(error(
                &self.path,
                io::ErrorKind::InvalidData,
                "The owned stage slot now denotes a foreign object.",
            ));
        }
        assert_eq!(
            self.control
                .namespace
                .lock()
                .unwrap()
                .stages
                .get(&self.value),
            Some(expected)
        );
        Ok(identity(self.value))
    }

    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.control.event("publish");
        self.control.publishes.fetch_add(1, Ordering::SeqCst);
        let path = self.destination.join(name);
        let mut namespace = self.control.namespace.lock().unwrap();
        if self.control.late_collision.swap(false, Ordering::SeqCst) {
            namespace.published.insert(
                path.clone(),
                Object {
                    entry: entry(path.clone(), EntryKind::Symlink, 90),
                    representation: Some(
                        TransferLinkRepresentation::linux_symbolic_link(b"winner/missing").unwrap(),
                    ),
                },
            );
        }
        if namespace.published.contains_key(&path) {
            return Err(error(
                &path,
                io::ErrorKind::AlreadyExists,
                "Late winner retained.",
            ));
        }
        let representation = namespace.stages.remove(&self.value).unwrap();
        namespace.published.insert(
            path.clone(),
            Object {
                entry: entry(path.clone(), EntryKind::Symlink, self.value),
                representation: Some(representation),
            },
        );
        self.published = Some(path.clone());
        if self.control.cancel_after_publish.load(Ordering::SeqCst) {
            // Cancellation cannot turn an actual native mutation into an ordinary error.
            cancel.cancel();
            self.control.event("cancel after publish");
        }
        Ok(
            match self.control.publication_outcome.load(Ordering::SeqCst) {
                1 => TransferPublication::PublishedDurabilityUnconfirmed {
                    path: path.clone(),
                    error: error(
                        &path,
                        io::ErrorKind::Other,
                        "Parent synchronization failed.",
                    ),
                },
                2 => TransferPublication::RecoveryRequired {
                    published: Some(path.clone()),
                    recovery: vec![TransferRecovery {
                        path: self.path.clone(),
                        identity: Some(identity(self.value)),
                    }],
                    error: error(
                        &path,
                        io::ErrorKind::Other,
                        "Post-publication recovery required.",
                    ),
                },
                _ => TransferPublication::Published(path),
            },
        )
    }

    fn revalidate_publication(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        self.control.event("publication binding");
        assert!(
            !cancel.is_cancelled(),
            "actual publication observation must still run"
        );
        let path = self.published.as_ref().unwrap();
        assert_eq!(self.control.output(path).as_ref(), Some(expected));
        Ok(
            if self.control.substitute_publication.load(Ordering::SeqCst) {
                identity(91)
            } else {
                identity(self.value)
            },
        )
    }

    fn cleanup(&mut self) -> Result<(), TransferLinkCleanupError> {
        self.control.cleanups.fetch_add(1, Ordering::SeqCst);
        let block = self.control.cleanup_block.lock().unwrap().take();
        if let Some(block) = block {
            block.entered.notify_one();
            block.release.recv_timeout(Duration::from_secs(4)).unwrap();
        }
        if self.published.is_some() {
            return Ok(());
        }
        let unconfirmed = self.control.cleanup_unconfirmed.load(Ordering::SeqCst)
            || self.control.substitute_stage.load(Ordering::SeqCst);
        if self.control.refuse_cleanup.load(Ordering::SeqCst) || unconfirmed {
            return Err(TransferLinkCleanupError {
                error: error(
                    &self.path,
                    io::ErrorKind::PermissionDenied,
                    "Owned link cleanup refused.",
                ),
                recovery: if unconfirmed {
                    vec![]
                } else {
                    vec![TransferRecovery {
                        path: self.path.clone(),
                        identity: Some(identity(self.value)),
                    }]
                },
                cleanup_unconfirmed: unconfirmed,
            });
        }
        self.control
            .namespace
            .lock()
            .unwrap()
            .stages
            .remove(&self.value);
        Ok(())
    }
}

pub(super) struct Fixture {
    pub(super) source: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) service: Arc<Service>,
    pub(super) control: Arc<Control>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::with_representation(relative_representation())
    }

    pub(super) fn with_two_roots() -> Self {
        let mut fixture = Self::new();
        let second = fixture.source.with_file_name("second");
        Arc::get_mut(&mut fixture.service)
            .unwrap()
            .roots
            .push(Node::link(second, relative_representation(), 2));
        fixture
    }

    pub(super) fn retain_registration(&self) -> Arc<Registration> {
        let registration = Arc::new(Registration {
            control: Arc::downgrade(&self.control),
        });
        *self.control.registration.lock().unwrap() = Arc::downgrade(&registration);
        registration
    }

    pub(super) fn with_representation(representation: TransferLinkRepresentation) -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join("_build/link-copy-fixture");
        let source = root.join("selected-link");
        let destination = root.join("destination");
        let control = Arc::new(Control::default());
        let service = Arc::new(Service {
            roots: vec![Node::link(source.clone(), representation, 1)],
            control: control.clone(),
        });
        Self {
            source,
            destination,
            service,
            control,
        }
    }

    pub(super) fn options(&self, id: u64) -> CopyOptionsRequest {
        CopyOptionsRequest {
            input: CopyOptionsInput::Native(MetadataTransferRequest {
                input: MetadataTransferInput::Paths(self.legacy(id)),
                intent: TransferIntent::Copy,
                metadata: TransferMetadataPolicy::ContentOnly,
            }),
            links: CopyLinkPolicy::Preserve,
        }
    }

    fn legacy(&self, id: u64) -> CopyRequest {
        CopyRequest {
            id: OperationId(id),
            clipboard: FileClipboard {
                paths: self
                    .service
                    .roots
                    .iter()
                    .map(|node| node.entry.item.path.clone())
                    .collect(),
                intent: FileClipboardIntent::Copy,
            },
            destination: self.destination.clone(),
            verification: TransferVerification::default(),
        }
    }

    fn prepare(&self) -> (worker::Job, Status, Cancellation) {
        let input = CopyInput::options(self.options(1));
        let status = Arc::new(Mutex::new(initial(&input)));
        let cancel = Cancellation::default();
        let mut preparation = preparation::Preparation::new(input, self.service.clone());
        let mut job = preparation.attempt(cancel.clone(), status.clone()).unwrap();
        bind_job_io(&mut job);
        (job, status, cancel)
    }

    pub(super) fn output(&self) -> Option<TransferLinkRepresentation> {
        self.control
            .output(&self.destination.join(self.source.file_name().unwrap()))
    }
}

fn bind_job_io(job: &mut worker::Job) {
    if !job.holds_io_permit() {
        let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        job.begin_io_step(Some(permit));
    }
}

fn finish(job: &mut worker::Job) {
    bind_job_io(job);
    for _ in 0..80 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => return,
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            worker::Step::Failure(failure) => panic!("unexpected failure: {failure:?}"),
        }
    }
    panic!("bounded link fixture did not finish");
}

fn next_failure(job: &mut worker::Job) -> Arc<CopyFailure> {
    bind_job_io(job);
    for _ in 0..80 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => return failure,
            worker::Step::Done => panic!("expected a versioned link failure"),
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    panic!("bounded link fixture did not reach failure");
}

fn next_conflict(job: &mut worker::Job) -> Arc<CopyConflict> {
    bind_job_io(job);
    for _ in 0..80 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Conflict(conflict) => return conflict,
            worker::Step::Done => panic!("expected a versioned link conflict"),
            worker::Step::Failure(failure) => panic!("unexpected failure: {failure:?}"),
        }
    }
    panic!("bounded link fixture did not reach conflict");
}

fn decide_failure(job: &mut worker::Job, failure: &CopyFailure, choice: CopyChoice) {
    assert!(job.accept_decision(CopyDecision {
        id: failure.id,
        version: failure.version,
        choice,
    }));
}

fn ready_to_publish(job: &mut worker::Job, status: &Status) {
    bind_job_io(job);
    for _ in 0..80 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(status).phase == CopyOperationPhase::Publishing {
            return;
        }
    }
    panic!("bounded link fixture did not reach publication");
}

#[test]
fn legacy_copy_rejects_links_without_creating_a_stage() {
    let fixture = Fixture::new();
    let input: CopyInput = fixture.legacy(1).into();
    assert_eq!(input.links, CopyLinkPolicy::Reject);
    let status = Arc::new(Mutex::new(initial(&input)));
    let mut preparation = preparation::Preparation::new(input, fixture.service.clone());
    match preparation.attempt(Cancellation::default(), status) {
        Ok(mut job) => {
            let failure = next_failure(&mut job);
            assert_eq!(failure.error.kind, io::ErrorKind::Unsupported);
            decide_failure(&mut job, &failure, CopyChoice::Cancel);
            assert!(matches!(job.step(), worker::Step::Done));
            assert_eq!(job.finish().files_completed, 0);
        }
        Err(error) => assert_eq!(error.kind, io::ErrorKind::Unsupported),
    }
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.referent_reads.load(Ordering::SeqCst), 0);
    assert!(fixture.output().is_none());
}

#[test]
fn preserve_copies_exact_raw_native_representations_as_leaves_with_no_content_credit() {
    let representations = [
        relative_representation(),
        TransferLinkRepresentation::linux_symbolic_link(b"self-cycle").unwrap(),
        TransferLinkRepresentation::windows_symbolic_link(
            true,
            1,
            &[46, 46, 92, 0xd800, 0x6587],
            &[46, 46, 92, 0xd800, 0x6587],
        )
        .unwrap(),
        TransferLinkRepresentation::windows_junction(
            &"\\??\\C:\\missing".encode_utf16().collect::<Vec<_>>(),
            &"C:\\missing".encode_utf16().collect::<Vec<_>>(),
        )
        .unwrap(),
    ];
    for representation in representations {
        let fixture = Fixture::with_representation(representation.clone());
        let (mut job, _, _) = fixture.prepare();
        // Link proof remains exact even under the sampled-content policy.
        job.set_verification(TransferVerification::SizeAndSamples);
        finish(&mut job);
        let done = job.finish();
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(fixture.output(), Some(representation));
        assert_eq!((done.files_completed, done.completed_roots), (1, 1));
        assert_eq!(
            (done.copied_bytes, done.verified_bytes, done.completed_bytes),
            (0, 0, 0)
        );
        assert_eq!(
            (done.source_files_removed, done.native_entries_moved),
            (0, 0)
        );
        assert_eq!(fixture.control.referent_reads.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.control.child_reads.load(Ordering::SeqCst), 0);
        assert_eq!(
            *fixture.control.policies.lock().unwrap(),
            vec![CopyLinkPolicy::Preserve]
        );
        let events = fixture.control.events.lock().unwrap();
        let position = |event| events.iter().position(|value| *value == event).unwrap();
        assert!(position("sync") < position("stage readback"));
        assert!(position("stage readback") < position("publish"));
        assert!(position("stage binding") < position("publish"));
        assert!(position("publish") < position("publication binding"));
        let last_source = events
            .iter()
            .rposition(|event| *event == "source representation")
            .unwrap();
        let last_stage = events
            .iter()
            .rposition(|event| *event == "stage binding")
            .unwrap();
        assert!(position("stage readback") < last_source);
        assert!(last_source < last_stage && last_stage < position("publish"));
        assert!(fixture.control.source_queries.load(Ordering::SeqCst) >= 2);
    }
}

#[test]
fn preserve_move_and_required_modified_time_reject_before_any_provider_preparation() {
    for (intent, metadata) in [
        (TransferIntent::Move, TransferMetadataPolicy::ContentOnly),
        (
            TransferIntent::Copy,
            TransferMetadataPolicy::RequireModifiedTime,
        ),
    ] {
        let fixture = Fixture::new();
        let mut options = fixture.options(1);
        let CopyOptionsInput::Native(request) = &mut options.input else {
            unreachable!()
        };
        request.intent = intent;
        request.metadata = metadata;
        let input = CopyInput::options(options);
        assert_eq!(
            input.validate().unwrap_err().kind,
            io::ErrorKind::Unsupported
        );
        let status = Arc::new(Mutex::new(initial(&input)));
        let mut preparation = preparation::Preparation::new(input, fixture.service.clone());
        let Err(error) = preparation.attempt(Cancellation::default(), status) else {
            panic!("incompatible Preserve was admitted")
        };
        assert_eq!(error.kind, io::ErrorKind::Unsupported);
        assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.control.destination_opens.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn unsupported_native_link_kind_rejects_at_admission_before_staging() {
    let fixture = Fixture::new();
    fixture
        .control
        .refuse_admission
        .store(true, Ordering::SeqCst);
    let input = CopyInput::options(fixture.options(1));
    let status = Arc::new(Mutex::new(initial(&input)));
    let mut preparation = preparation::Preparation::new(input, fixture.service.clone());
    let Err(error) = preparation.attempt(Cancellation::default(), status) else {
        panic!("unsupported native link kind was admitted")
    };
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.referent_reads.load(Ordering::SeqCst), 0);
}

#[test]
fn link_staging_without_an_actual_copy_io_permit_rejects_before_creation() {
    let fixture = Fixture::new();
    let input = CopyInput::options(fixture.options(1));
    let status = Arc::new(Mutex::new(initial(&input)));
    let mut preparation = preparation::Preparation::new(input, fixture.service.clone());
    let mut job = preparation
        .attempt(Cancellation::default(), status)
        .unwrap();
    assert!(!job.holds_io_permit());
    for _ in 0..80 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => {
                assert_eq!(failure.error.kind, io::ErrorKind::InvalidInput);
                assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
                assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 0);
                decide_failure(&mut job, &failure, CopyChoice::Cancel);
                assert!(matches!(job.step(), worker::Step::Done));
                assert_eq!(job.finish().files_completed, 0);
                return;
            }
            worker::Step::Done => panic!("missing Copy permit must be an explicit failure"),
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    panic!("bounded fixture did not reject unleased staging");
}

#[test]
fn preserve_regular_file_keeps_independent_content_readback_and_byte_accounting() {
    let mut fixture = Fixture::new();
    let payload = b"ordinary file content".to_vec();
    fixture.source = fixture.source.with_file_name("ordinary-file");
    let mut file = entry(fixture.source.clone(), EntryKind::File, 1);
    file.item.metadata.size = Some(payload.len() as u64);
    Arc::get_mut(&mut fixture.service).unwrap().roots = vec![Node {
        entry: file,
        representation: None,
        children: vec![],
    }];
    *fixture.control.regular_payload.lock().unwrap() = Some(payload.clone());
    let (mut job, _, _) = fixture.prepare();
    finish(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.completed_roots, 1);
    assert_eq!(
        (done.copied_bytes, done.verified_bytes, done.completed_bytes),
        (21, 21, 21)
    );
    assert_eq!(fixture.control.regular_reads.load(Ordering::SeqCst), 21);
    assert_eq!(fixture.control.file_readbacks.load(Ordering::SeqCst), 21);
    assert_eq!(fixture.control.file_stages.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.source_queries.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.referent_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .control
            .namespace
            .lock()
            .unwrap()
            .files
            .get(&fixture.destination.join("ordinary-file")),
        Some(&payload)
    );
}

#[test]
fn preserve_preflights_unsupported_later_roots_before_any_batch_writes() {
    let mut fixture = Fixture::new();
    let unsupported = fixture.source.with_file_name("unsupported-root");
    Arc::get_mut(&mut fixture.service)
        .unwrap()
        .roots
        .push(Node {
            entry: entry(unsupported, EntryKind::Other, 2),
            representation: None,
            children: vec![],
        });
    let input = CopyInput::options(fixture.options(1));
    let status = Arc::new(Mutex::new(initial(&input)));
    let mut preparation = preparation::Preparation::new(input, fixture.service.clone());
    let Err(error) = preparation.attempt(Cancellation::default(), status) else {
        panic!("unsupported later root was admitted")
    };
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.file_stages.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .control
            .namespace
            .lock()
            .unwrap()
            .published
            .is_empty()
    );
}

#[test]
fn independent_stage_readback_mismatch_never_publishes() {
    let fixture = Fixture::new();
    fixture
        .control
        .mismatch_readback
        .store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    let failure = next_failure(&mut job);
    assert_eq!(failure.error.kind, io::ErrorKind::InvalidData);
    assert!(!failure.can_retry);
    decide_failure(&mut job, &failure, CopyChoice::Skip);
    finish(&mut job);
    assert_eq!(job.finish().files_completed, 0);
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
    assert!(fixture.control.namespace.lock().unwrap().stages.is_empty());
    assert!(fixture.output().is_none());
}

#[test]
fn retained_source_name_binding_is_checked_again_after_stage_readback() {
    let fixture = Fixture::new();
    fixture
        .control
        .change_binding_after_readback
        .store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    let failure = next_failure(&mut job);
    assert_eq!(failure.error.kind, io::ErrorKind::InvalidData);
    decide_failure(&mut job, &failure, CopyChoice::Cancel);
    assert!(matches!(job.step(), worker::Step::Done));
    assert_eq!(job.finish().files_completed, 0);
    assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
}

#[test]
fn same_identity_source_payload_change_invalidates_the_raw_link_proof() {
    let fixture = Fixture::new();
    fixture
        .control
        .change_payload_after_readback
        .store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    let failure = next_failure(&mut job);
    assert_eq!(failure.error.kind, io::ErrorKind::InvalidData);
    decide_failure(&mut job, &failure, CopyChoice::Cancel);
    assert!(matches!(job.step(), worker::Step::Done));
    assert_eq!(job.finish().files_completed, 0);
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
}

#[test]
fn substituted_stage_is_neither_published_nor_deleted_as_an_owned_residual() {
    let fixture = Fixture::new();
    let (mut job, status, _) = fixture.prepare();
    ready_to_publish(&mut job, &status);
    fixture
        .control
        .substitute_stage
        .store(true, Ordering::SeqCst);
    let failure = next_failure(&mut job);
    decide_failure(&mut job, &failure, CopyChoice::Cancel);
    assert!(matches!(job.step(), worker::Step::Done));
    let done = job.finish();
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
    assert_eq!(
        done.recovery_count, 0,
        "foreign identity is not a verified residual"
    );
    assert!(!fixture.control.namespace.lock().unwrap().stages.is_empty());
    assert!(
        done.errors
            .iter()
            .any(|error| error.path.ends_with(".owned-link-100"))
    );
    assert!(
        done.message.to_lowercase().contains("unconfirmed"),
        "{done:?}"
    );
}

#[test]
fn changed_published_identity_keeps_actual_publication_and_reports_failed_proof() {
    let fixture = Fixture::new();
    fixture
        .control
        .substitute_publication
        .store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    finish(&mut job);
    let done = job.finish();
    assert_eq!(done.files_completed, 1);
    assert!(done.failed > 0, "{done:?}");
    assert_eq!(fixture.output(), Some(relative_representation()));
    assert_eq!(done.completed_roots, 0);
    assert_eq!(done.source_files_removed, 0);
}

#[test]
fn late_link_conflict_retries_the_same_pins_and_stage_with_versioned_no_replace() {
    let fixture = Fixture::new();
    fixture.control.late_collision.store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    let first = next_conflict(&mut job);
    assert!(!first.can_replace && !first.can_merge);
    for choice in [
        CopyChoice::ReplaceRecoverable { all: false },
        CopyChoice::ReplaceRecoverable { all: true },
        CopyChoice::Merge { all: false },
        CopyChoice::Merge { all: true },
    ] {
        assert!(!job.accept_decision(CopyDecision {
            id: first.id,
            version: first.version,
            choice,
        }));
    }
    assert!(job.accept_decision(CopyDecision {
        id: first.id,
        version: first.version,
        choice: CopyChoice::Retry,
    }));
    let second = next_conflict(&mut job);
    assert!(second.version > first.version);
    assert!(!job.accept_decision(CopyDecision {
        id: first.id,
        version: first.version,
        choice: CopyChoice::Skip,
    }));
    assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .control
            .output(&second.destination)
            .unwrap()
            .linux_target(),
        Some(b"winner/missing".as_slice())
    );
    fixture
        .control
        .namespace
        .lock()
        .unwrap()
        .published
        .remove(&second.destination);
    assert!(job.accept_decision(CopyDecision {
        id: second.id,
        version: second.version,
        choice: CopyChoice::Retry,
    }));
    finish(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(fixture.output(), Some(relative_representation()));
    assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 1);
}

#[test]
fn late_link_conflict_skip_and_cancel_keep_the_dangling_winner_and_clean_stage() {
    for cancel in [false, true] {
        let fixture = Fixture::new();
        fixture.control.late_collision.store(true, Ordering::SeqCst);
        let (mut job, _, _) = fixture.prepare();
        let conflict = next_conflict(&mut job);
        let winner = fixture.output().unwrap();
        assert!(job.accept_decision(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: if cancel {
                CopyChoice::Cancel
            } else {
                CopyChoice::Skip
            },
        }));
        finish(&mut job);
        let done = job.finish();
        assert_eq!(done.files_completed, 0);
        assert_eq!(done.skipped, u64::from(!cancel));
        assert_eq!(fixture.output(), Some(winner));
        assert!(fixture.control.namespace.lock().unwrap().stages.is_empty());
        assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn cancel_before_link_publication_cleans_the_stage_and_keeps_the_source() {
    let fixture = Fixture::new();
    let (mut job, status, cancel) = fixture.prepare();
    ready_to_publish(&mut job, &status);
    cancel.cancel();
    assert!(matches!(job.step(), worker::Step::Done));
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert_eq!((done.files_completed, done.source_files_removed), (0, 0));
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
    assert!(fixture.control.namespace.lock().unwrap().stages.is_empty());
}

#[test]
fn cancellation_after_actual_link_publication_keeps_the_publication_fact() {
    let fixture = Fixture::new();
    fixture
        .control
        .cancel_after_publish
        .store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    finish(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.files_completed, 1);
    assert_eq!(
        done.failed, 0,
        "late cancel must not prevent the owned publication check"
    );
    assert_eq!(done.source_files_removed, 0);
    assert_eq!(fixture.output(), Some(relative_representation()));
}

#[test]
fn structured_post_publication_failures_keep_actual_destinations_and_recovery_facts() {
    for outcome in [1, 2] {
        let fixture = Fixture::new();
        fixture
            .control
            .publication_outcome
            .store(outcome, Ordering::SeqCst);
        let (mut job, _, _) = fixture.prepare();
        finish(&mut job);
        let done = job.finish();
        assert_eq!(done.files_completed, 1, "{done:?}");
        assert!(done.failed > 0, "{done:?}");
        assert_eq!(fixture.output(), Some(relative_representation()));
        assert_eq!(done.source_files_removed, 0);
        assert_eq!(done.recovery_count, u64::from(outcome == 2));
        if outcome == 2 {
            assert_eq!(
                done.recovery_paths,
                vec![fixture.destination.join(".owned-link-100")]
            );
        }
    }
}

#[test]
fn preparation_retry_retains_policy_and_never_reopens_an_earlier_link_pin() {
    let mut fixture = Fixture::new();
    let second = fixture.source.with_file_name("second");
    Arc::get_mut(&mut fixture.service)
        .unwrap()
        .roots
        .push(Node::link(second.clone(), relative_representation(), 2));
    fixture
        .control
        .fail_second_open
        .store(true, Ordering::SeqCst);
    let input = CopyInput::options(fixture.options(1));
    let status = Arc::new(Mutex::new(initial(&input)));
    let mut preparation = preparation::Preparation::new(input, fixture.service.clone());
    let Err(error) = preparation.attempt(Cancellation::default(), status.clone()) else {
        panic!("second source fixture should refuse the first preparation")
    };
    assert_eq!(error.kind, io::ErrorKind::NotConnected);
    assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 2);
    let replacement = TransferLinkRepresentation::linux_symbolic_link(b"newly-opened").unwrap();
    *fixture.control.replacement_on_open.lock().unwrap() = Some(replacement.clone());
    let mut job = preparation
        .attempt(Cancellation::default(), status)
        .unwrap();
    drop(preparation);
    finish(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 3);
    assert_eq!(fixture.control.legacy_opens.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .control
            .policies
            .lock()
            .unwrap()
            .iter()
            .all(|policy| *policy == CopyLinkPolicy::Preserve)
    );
    assert_eq!(fixture.output(), Some(relative_representation()));
    assert_eq!(
        fixture.control.output(&fixture.destination.join("second")),
        Some(replacement)
    );
}

#[test]
fn recursive_links_are_leaves_and_later_unsupported_children_keep_completed_copy_results() {
    let mut fixture = Fixture::new();
    let parent = fixture.source.with_file_name("folder");
    let link = parent.join("child-link");
    Arc::get_mut(&mut fixture.service).unwrap().roots = vec![Node {
        entry: entry(parent.clone(), EntryKind::Directory, 3),
        representation: None,
        children: vec![
            Node::link(link, relative_representation(), 4),
            Node {
                entry: entry(parent.join("unsupported"), EntryKind::Other, 5),
                representation: None,
                children: vec![],
            },
        ],
    }];
    let (mut job, _, _) = fixture.prepare();
    let failure = next_failure(&mut job);
    assert_eq!(failure.error.kind, io::ErrorKind::Unsupported);
    decide_failure(&mut job, &failure, CopyChoice::Skip);
    finish(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::CompletedWithErrors);
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.completed_roots, 0);
    assert_eq!(
        fixture
            .control
            .output(&fixture.destination.join("folder/child-link")),
        Some(relative_representation())
    );
    assert_eq!(fixture.control.referent_reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 1);
}

#[test]
fn partially_created_link_stage_is_retained_for_cleanup_and_cannot_be_retried() {
    let fixture = Fixture::new();
    fixture
        .control
        .fail_stage_creation
        .store(true, Ordering::SeqCst);
    let (mut job, _, _) = fixture.prepare();
    let failure = next_failure(&mut job);
    assert!(!failure.can_retry);
    assert!(!job.accept_decision(CopyDecision {
        id: failure.id,
        version: failure.version,
        choice: CopyChoice::Retry,
    }));
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.drops.lock().unwrap().len(), 0);
    decide_failure(&mut job, &failure, CopyChoice::Skip);
    finish(&mut job);
    assert_eq!(job.finish().files_completed, 0);
    assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
    assert!(fixture.control.namespace.lock().unwrap().stages.is_empty());
}

#[test]
fn refused_link_cleanup_stops_automatic_progress_and_reports_only_verified_residuals() {
    for unconfirmed in [false, true] {
        let mut fixture = Fixture::new();
        let second = fixture.source.with_file_name("second");
        Arc::get_mut(&mut fixture.service)
            .unwrap()
            .roots
            .push(Node::link(second, relative_representation(), 2));
        let registration = fixture.retain_registration();
        fixture
            .control
            .fail_stage_creation
            .store(true, Ordering::SeqCst);
        fixture.control.refuse_cleanup.store(true, Ordering::SeqCst);
        fixture
            .control
            .cleanup_unconfirmed
            .store(unconfirmed, Ordering::SeqCst);
        let (mut job, _, _) = fixture.prepare();
        drop(registration);
        let failure = next_failure(&mut job);
        assert!(!failure.can_retry);
        decide_failure(&mut job, &failure, CopyChoice::Skip);
        finish(&mut job);
        let done = job.finish();
        assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.control.publishes.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(done.recovery_count, u64::from(!unconfirmed), "{done:?}");
        if unconfirmed {
            assert!(
                done.message.to_lowercase().contains("unconfirmed"),
                "{done:?}"
            );
            assert!(done.recovery_paths.is_empty());
        } else {
            assert_eq!(
                done.recovery_paths,
                vec![fixture.destination.join(".owned-link-100")]
            );
        }
        assert!(!fixture.control.namespace.lock().unwrap().stages.is_empty());
        let drops = fixture.control.drops.lock().unwrap();
        assert!(
            drops.contains(&("stage", true)),
            "stage dropped after provider: {drops:?}"
        );
        let stage = drops
            .iter()
            .position(|(owner, _)| *owner == "stage")
            .unwrap();
        let source = drops
            .iter()
            .position(|(owner, _)| *owner == "source")
            .unwrap();
        assert!(
            stage < source,
            "source registration must outlive stage destruction: {drops:?}"
        );
        assert!(
            fixture
                .control
                .registration
                .lock()
                .unwrap()
                .upgrade()
                .is_none()
        );
    }
}
