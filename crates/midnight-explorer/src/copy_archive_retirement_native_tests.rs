//! Installed archive retirement through the real Engine and native transfer service.
use super::*;
use midnight_core::*;
use midnight_plugin_host::{CapturedFile, ContentSource};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::Path,
    sync::{
        Condvar,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
const DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct Hold {
    entered: AtomicBool,
    released: Mutex<bool>,
    changed: Condvar,
    timed_out: AtomicBool,
}
impl Hold {
    fn once(&self) {
        if self.entered.swap(true, Ordering::AcqRel) {
            return;
        }
        let released = self
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (released, timeout) = self
            .changed
            .wait_timeout_while(released, DEADLINE, |released| !*released)
            .unwrap_or_else(|error| error.into_inner());
        if timeout.timed_out() && !*released {
            self.timed_out.store(true, Ordering::Release);
        }
    }
    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        self.changed.notify_all();
    }
}
struct ReleaseAll(Vec<Arc<Hold>>);
impl Drop for ReleaseAll {
    fn drop(&mut self) {
        for hold in &self.0 {
            hold.release();
        }
    }
}

// An error or unwind retains any unconfirmed native tail. The successful path
// explicitly extracts only after its consuming children have returned.
struct Pending<T>(Option<T>);
impl<T> Pending<T> {
    fn get(&self) -> &T {
        self.0.as_ref().unwrap()
    }
    fn get_mut(&mut self) -> &mut T {
        self.0.as_mut().unwrap()
    }
    fn take(&mut self) -> T {
        self.0.take().unwrap()
    }
}
impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::mem::forget(value);
        }
    }
}

#[derive(Debug, Default)]
struct CloseState {
    hold: Arc<Hold>,
    inner_returned: AtomicBool,
    file_closed: AtomicBool,
    compressed_reads: AtomicU64,
}
#[derive(Debug)]
struct HeldContent {
    source: Option<CapturedFile>,
    file: Option<File>,
    state: Arc<CloseState>,
}
impl HeldContent {
    fn source(&self) -> &CapturedFile {
        self.source.as_ref().unwrap()
    }
}
impl ContentSource for HeldContent {
    fn length(&self) -> u64 {
        self.source().length()
    }
    fn display_name(&self) -> String {
        self.source().display_name()
    }
    fn version(&self) -> Vec<u8> {
        self.source().stable_version()
    }
    fn validate(&self) -> std::result::Result<(), midnight_plugin_host::Error> {
        self.source().revalidate()
    }
    fn read_at(
        &self,
        offset: u64,
        length: u32,
    ) -> std::result::Result<Vec<u8>, midnight_plugin_host::Error> {
        let bytes = self.source().read_at(offset, length)?;
        self.state
            .compressed_reads
            .fetch_add(bytes.len() as u64, Ordering::AcqRel);
        Ok(bytes)
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.source().cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> std::result::Result<(), midnight_plugin_host::Error> {
        if self.cleanup_unconfirmed() {
            std::mem::forget(self);
            return Err(midnight_plugin_host::Error(
                "observed native source cleanup refusal".into(),
            ));
        }
        let Some(owner) = Arc::into_inner(self) else {
            return Ok(());
        };
        let mut owner = Pending(Some(owner));
        owner.get_mut().source.take().unwrap().release_on_worker()?;
        owner
            .get()
            .state
            .inner_returned
            .store(true, Ordering::Release);
        // The installed Content/Snapshot consuming bridge has returned, but this
        // last native File owner has not yet closed. The archive host must wait.
        owner.get().state.hold.once();
        drop(owner.get_mut().file.take());
        owner.get().state.file_closed.store(true, Ordering::Release);
        drop(owner.take());
        Ok(())
    }
}

fn held_capture(
    provider: Arc<InstalledProvider>,
    state: Arc<CloseState>,
) -> midnight_engine::ArchiveCapture {
    let capture = archive(provider);
    Arc::new(move |path, expected, cancel| {
        let mut source = Pending(Some(capture(path, expected, cancel)?));
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) => {
                source.take().release_on_worker().map_err(|cleanup| {
                    FsError::from_io(
                        "release fixture capture",
                        path,
                        io::Error::other(cleanup.to_string()),
                    )
                })?;
                return Err(FsError::from_io("open fixture close witness", path, error));
            }
        };
        let wrapper = Arc::new(HeldContent {
            source: Some(source.take()),
            file: Some(file),
            state: state.clone(),
        });
        let mut retained = Pending(Some(wrapper.clone()));
        let result = CapturedFile::from_source(wrapper);
        retained.take().release_on_worker().map_err(|error| {
            FsError::from_io(
                "release fixture construction owner",
                path,
                io::Error::other(error.to_string()),
            )
        })?;
        result.map_err(|error| {
            FsError::from_io(
                "wrap native capture",
                path,
                io::Error::other(error.to_string()),
            )
        })
    })
}

#[derive(Debug)]
struct ReadHoldMount {
    source: Option<Arc<dyn ArchiveMount>>,
    hold: Arc<Hold>,
}
impl ReadHoldMount {
    fn source(&self) -> &Arc<dyn ArchiveMount> {
        self.source.as_ref().unwrap()
    }
}
impl ArchiveMount for ReadHoldMount {
    fn id(&self) -> u64 {
        self.source().id()
    }
    fn validate(&self) -> std::result::Result<(), FsError> {
        self.source().validate()
    }
    fn list(
        &self,
        directory: u64,
        cancel: &Cancellation,
    ) -> std::result::Result<Vec<ArchiveNode>, FsError> {
        self.source().list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> std::result::Result<Vec<u8>, FsError> {
        let bytes = self.source().read(entry, offset, length, cancel)?;
        if !bytes.is_empty() {
            self.hold.once();
        }
        Ok(bytes)
    }
    fn native_container_locator(&self) -> Option<&Path> {
        self.source().native_container_locator()
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        version: &[u8],
    ) -> std::result::Result<Option<Vec<u8>>, FsError> {
        self.source().persistent_member_identity(entry, version)
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.source().cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> std::result::Result<(), FsError> {
        if self.cleanup_unconfirmed() {
            std::mem::forget(self);
            return Err(FsError::from_io(
                "release read witness",
                Path::new(""),
                io::Error::other("archive cleanup unconfirmed"),
            ));
        }
        let Some(owner) = Arc::into_inner(self) else {
            return Ok(());
        };
        let mut owner = Pending(Some(owner));
        owner.get_mut().source.take().unwrap().release_on_worker()?;
        drop(owner.take());
        Ok(())
    }
}

// Every capability comes from this exact concrete LocalFileSystem. The wrapper
// only observes successful native reads and destroys its delegates synchronously.
struct Transfers {
    native: midnight_platform::LocalFileSystem,
    reads: Arc<AtomicU64>,
}
impl AuditedSynchronousTransferService for Transfers {}
impl TransferService for Transfers {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> std::result::Result<Box<dyn TransferSource>, FsError> {
        self.native
            .prepare_transfer_source(path, cancel)
            .map(|source| ObservedSource::wrap(source, self.reads.clone()))
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> std::result::Result<Arc<dyn TransferDirectory>, FsError> {
        self.native.prepare_checksum_directory(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> std::result::Result<Arc<dyn TransferDirectory>, FsError> {
        self.native.prepare_transfer_destination(path, cancel)
    }
}
struct ObservedSource {
    source: Box<dyn TransferSource>,
    reads: Arc<AtomicU64>,
}
impl ObservedSource {
    fn wrap(source: Box<dyn TransferSource>, reads: Arc<AtomicU64>) -> Box<dyn TransferSource> {
        Box::new(Self { source, reads })
    }
}
impl TransferSource for ObservedSource {
    fn entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.source.native_identity()
    }
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> std::result::Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.source.native_namespace_scope(cancel)
    }
    fn revalidate(&self, cancel: &Cancellation) -> std::result::Result<(), FsError> {
        self.source.revalidate(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> std::result::Result<(), FsError> {
        self.source.discover(cancel, visit)
    }
    fn read_chunk(
        &mut self,
        buffer: &mut [u8],
        cancel: &Cancellation,
    ) -> std::result::Result<usize, FsError> {
        let count = self.source.read_chunk(buffer, cancel)?;
        self.reads.fetch_add(count as u64, Ordering::AcqRel);
        Ok(count)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> std::result::Result<Option<Box<dyn TransferSource>>, FsError> {
        self.source
            .next_child(cancel)
            .map(|child| child.map(|source| Self::wrap(source, self.reads.clone())))
    }
    fn open_child(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> std::result::Result<Box<dyn TransferSource>, FsError> {
        self.source
            .open_child(name, cancel)
            .map(|source| Self::wrap(source, self.reads.clone()))
    }
    fn supports_read_retry(&self) -> bool {
        self.source.supports_read_retry()
    }
    fn retry_read_from(
        &mut self,
        offset: u64,
        cancel: &Cancellation,
    ) -> std::result::Result<(), FsError> {
        self.source.retry_read_from(offset, cancel)
    }
    fn seek_for_validation(
        &mut self,
        offset: u64,
        cancel: &Cancellation,
    ) -> std::result::Result<u64, FsError> {
        self.source.seek_for_validation(offset, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> std::result::Result<Box<dyn TransferReadGuard>, FsError> {
        self.source.rewind_for_validation(cancel)
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> std::result::Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.source.prepare_modified_time(cancel)
    }
}

struct NativeTempDir(PathBuf);
impl NativeTempDir {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "midnight-archive-close-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}
// Cleanup is explicit after Engine shutdown acknowledges actual worker teardown.
// A missing acknowledgement retains this owned directory for diagnosis.

struct Session {
    commands: midnight_engine::CommandSender,
    events: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
    progress: HashMap<OperationId, Arc<CopyProgress>>,
    terminal: HashMap<OperationId, Arc<CopyProgress>>,
    native_navigation: HashSet<NavigationId>,
    waiting: HashSet<OperationId>,
    errors: Vec<String>,
}
impl Session {
    fn start(
        provider: Arc<InstalledProvider>,
        capture: midnight_engine::ArchiveCapture,
        reads: Arc<AtomicU64>,
    ) -> Self {
        let (sender, events) = mpsc::channel(128);
        let mut engine = Engine::new(
            Engine::lifetime().expect("native fixture cleanup worker"),
            provider,
            sender,
        );
        let (commands, receiver) = engine.command_channel(32).unwrap();
        engine.configure_archive_capture().unwrap().install(capture);
        engine
            .configure_audited_transfers()
            .unwrap()
            .install(Arc::new(Transfers {
                native: midnight_platform::LocalFileSystem::new(None),
                reads,
            }));
        Self {
            commands,
            events,
            task: tokio::spawn(engine.run(receiver)),
            progress: HashMap::new(),
            terminal: HashMap::new(),
            native_navigation: HashSet::new(),
            waiting: HashSet::new(),
            errors: Vec::new(),
        }
    }
    async fn send(&self, command: Command) -> Result<()> {
        tokio::time::timeout(DEADLINE, self.commands.send(command))
            .await
            .map_err(|_| "command send timed out".to_owned())?
            .map_err(|_| "engine command channel closed".to_owned())
    }
    fn observe(&mut self, event: Event) {
        match event {
            Event::Copy(CopyEvent::Progress(progress)) => {
                self.progress.insert(progress.id, progress.clone());
                if progress.wait_reason == Some(TransferWaitReason::EarlierTransfers) {
                    self.waiting.insert(progress.id);
                }
                if progress.state.is_terminal() {
                    self.terminal.insert(progress.id, progress);
                }
            }
            Event::Copy(CopyEvent::Failure(failure)) => self
                .errors
                .push(format!("unexpected Copy failure: {failure:?}")),
            Event::Copy(CopyEvent::Conflict(conflict)) => self
                .errors
                .push(format!("unexpected Copy conflict: {conflict:?}")),
            Event::ListingFinished { navigation, .. } => {
                self.native_navigation.insert(navigation);
            }
            Event::ListingFailed { error, .. } => self.errors.push(error.to_string()),
            _ => {}
        }
    }
    async fn pump(&mut self, duration: Duration) -> Result<()> {
        match tokio::time::timeout(duration, self.events.recv()).await {
            Ok(Some(event)) => self.observe(event),
            Ok(None) => return Err("engine event stream closed".into()),
            Err(_) => {}
        }
        if let Some(error) = self.errors.first() {
            return Err(error.clone());
        }
        Ok(())
    }
    async fn until(&mut self, ready: impl Fn(&Self) -> bool) -> Result<()> {
        let end = Instant::now() + DEADLINE;
        while !ready(self) {
            if Instant::now() >= end {
                return Err("native retirement observation timed out".into());
            }
            self.pump(Duration::from_millis(10)).await?;
        }
        Ok(())
    }
    async fn archive(
        &mut self,
        pane: PaneId,
        navigation: u64,
        action: ArchiveAction,
    ) -> Result<ArchiveListing> {
        self.send(Command::ArchiveNavigate {
            pane,
            navigation: NavigationId(navigation),
            action,
            sort: SortSpec::default(),
            replaces: None,
        })
        .await?;
        let end = Instant::now() + DEADLINE;
        loop {
            let remaining = end.saturating_duration_since(Instant::now());
            let event = tokio::time::timeout(remaining, self.events.recv())
                .await
                .map_err(|_| "archive navigation timed out".to_owned())?
                .ok_or_else(|| "archive navigation event stream closed".to_owned())?;
            match event {
                Event::ArchiveListed {
                    navigation: found,
                    result,
                    ..
                } if found == NavigationId(navigation) => {
                    return result.map_err(|error| error.to_string());
                }
                other => self.observe(other),
            }
        }
    }
    async fn navigate_native(&self, pane: PaneId, navigation: u64, path: &Path) -> Result<()> {
        self.send(Command::Navigate {
            pane,
            navigation: NavigationId(navigation),
            path: path.to_owned(),
            sort: SortSpec::default(),
        })
        .await
    }
    async fn stop(mut self) -> Result<()> {
        // Cancellation cannot release fixture holds. The caller releases them
        // first; shutdown then drains actual workers while events keep flowing.
        let _ = self.send(Command::CancelCopy(OperationId(1))).await;
        let _ = self.send(Command::CancelCopy(OperationId(2))).await;
        self.send(Command::Shutdown).await?;
        let end = tokio::time::Instant::now() + DEADLINE;
        loop {
            tokio::select! {
                result = &mut self.task => return result.map_err(|error| error.to_string()),
                event = self.events.recv() => if let Some(event) = event { self.observe(event); },
                _ = tokio::time::sleep_until(end) => return Err("Engine shutdown did not acknowledge worker teardown".into()),
            }
        }
    }
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

async fn exercise(
    session: &mut Session,
    root: &Path,
    close: &Arc<CloseState>,
    read_hold: &Arc<Hold>,
    native_reads: &Arc<AtomicU64>,
    shared_pane: bool,
) -> Result<()> {
    let listing = session
        .archive(
            PaneId::Left,
            1,
            ArchiveAction::Open {
                path: root.join("source.gz"),
                format: ArchiveFormat::Gzip,
            },
        )
        .await?;
    require(
        listing.entries.len() == 1,
        "gzip fixture must expose exactly one member",
    )?;
    let entry = &listing.entries[0];
    let node = entry
        .virtual_node
        .as_ref()
        .ok_or_else(|| "archive entry has no native plugin identity".to_owned())?;
    let size = node
        .size
        .ok_or_else(|| "archive member size unavailable".to_owned())?;
    require(
        size > 0 && size <= 1024 * 1024,
        "fixture member must be a nonempty bounded file",
    )?;
    let member = node.id.entry;
    let name = entry.name.clone();
    let mount = listing
        .location
        .mount
        .as_ref()
        .ok_or_else(|| "archive mount unavailable".to_owned())?
        .clone();
    let expected = tokio::task::spawn_blocking(move || {
        let bytes = mount.read(member, 0, size as u32, &Cancellation::default());
        // This temporary reference is explicitly released; the pane still owns
        // the canonical mount, so this does not acquire global close responsibility.
        mount
            .release_on_worker()
            .map_err(|error| error.to_string())?;
        bytes.map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    require(
        expected.len() as u64 == size,
        "native archive returned a short reference read",
    )?;
    if shared_pane {
        let second = session
            .archive(
                PaneId::Right,
                2,
                ArchiveAction::List(listing.location.clone()),
            )
            .await?;
        // Only the actual second pane root remains; no test Arc substitutes for it.
        drop(second);
    }
    let mut location = listing.location;
    location.mount = Some(Arc::new(ReadHoldMount {
        source: location.mount.take(),
        hold: read_hold.clone(),
    }));
    let request = ArchiveCopyRequest {
        id: OperationId(1),
        start_policy: TransferStartPolicy::Immediate,
        verification: TransferVerification::FullSha256,
        location,
        selected: Arc::new(listing.entries.iter().map(|entry| entry.id).collect()),
        entries: listing.entries,
        view: listing.index,
        fallback: None,
        destination: root.join("archive-output"),
    };
    session.send(Command::EnqueueArchiveCopy(request)).await?;
    session
        .until(|_| read_hold.entered.load(Ordering::Acquire))
        .await?;
    require(
        !read_hold.timed_out.load(Ordering::Acquire),
        "decompressed read hold timed out",
    )?;
    // Drop the actual pane ReadyRoot while the Copy worker owns a successful
    // native decompressed read. The native listing event follows old-root Drop.
    session.navigate_native(PaneId::Left, 3, root).await?;
    session
        .until(|state| state.native_navigation.contains(&NavigationId(3)))
        .await?;
    session
        .send(Command::EnqueueCopy(CopyRequest {
            id: OperationId(2),
            start_policy: TransferStartPolicy::WaitForEarlier,
            verification: TransferVerification::FullSha256,
            clipboard: FileClipboard {
                paths: vec![root.join("later-source.bin")],
                intent: FileClipboardIntent::Copy,
            },
            destination: root.join("later-output"),
        }))
        .await?;
    session
        .until(|state| state.waiting.contains(&OperationId(2)))
        .await?;
    require(
        native_reads.load(Ordering::Acquire) == 0,
        "dependent native Copy read before its predecessor finished",
    )?;
    read_hold.release();

    if shared_pane {
        // A clean shared archive release must not wait for the other pane's
        // global mount lifetime. B's actual read proves A's FINISHED was settled.
        session
            .until(|state| {
                state.terminal.contains_key(&OperationId(1))
                    && state.terminal.contains_key(&OperationId(2))
            })
            .await?;
        require(
            native_reads.load(Ordering::Acquire) > 0,
            "dependent Copy never reached real native read",
        )?;
        require(
            !close.hold.entered.load(Ordering::Acquire),
            "Copy waited for the still-open second pane's global mount",
        )?;
        session.navigate_native(PaneId::Right, 4, root).await?;
        session
            .until(|_| close.hold.entered.load(Ordering::Acquire))
            .await?;
    } else {
        session
            .until(|_| close.hold.entered.load(Ordering::Acquire))
            .await?;
        require(
            close.inner_returned.load(Ordering::Acquire),
            "held close did not pass the actual native Content release",
        )?;
        require(
            !close.file_closed.load(Ordering::Acquire),
            "actual held File closed before release",
        )?;
        // Pump both jobs' events together; do not discard one terminal while
        // waiting for the other. A bounded absence observation is paired with
        // explicit hold-entry/close acknowledgements and positive completion.
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            session.pump(Duration::from_millis(10)).await?;
        }
        require(
            !session.terminal.contains_key(&OperationId(1)),
            "archive Copy terminal preceded actual File close",
        )?;
        require(
            !session.terminal.contains_key(&OperationId(2)),
            "dependent Copy terminal preceded actual File close",
        )?;
        require(
            session
                .progress
                .get(&OperationId(2))
                .is_some_and(|progress| {
                    progress.wait_reason == Some(TransferWaitReason::EarlierTransfers)
                }),
            "dependent Copy advanced past FINISHED waiting while the actual File remained open",
        )?;
        require(
            native_reads.load(Ordering::Acquire) == 0,
            "dependent native read preceded predecessor native close",
        )?;
    }
    close.hold.release();
    session
        .until(|state| {
            close.file_closed.load(Ordering::Acquire)
                && state.terminal.contains_key(&OperationId(1))
                && state.terminal.contains_key(&OperationId(2))
        })
        .await?;
    if shared_pane {
        session
            .until(|state| state.native_navigation.contains(&NavigationId(4)))
            .await?;
    }
    require(
        !close.hold.timed_out.load(Ordering::Acquire),
        "native File release hold timed out",
    )?;
    require(
        close.compressed_reads.load(Ordering::Acquire) > 0,
        "installed archive did not read actual native compressed bytes",
    )?;
    require(
        native_reads.load(Ordering::Acquire) > 0,
        "dependent Copy did not delegate real native reads",
    )?;
    for (id, length) in [(OperationId(1), size), (OperationId(2), 4096)] {
        let progress = &session.terminal[&id];
        require(
            progress.state == CopyOperationState::Completed
                && progress.failed == 0
                && progress.skipped == 0
                && progress.errors.is_empty(),
            &format!("unexpected final Copy result: {progress:?}"),
        )?;
        require(
            progress.files_completed == 1
                && progress.copied_bytes == length
                && progress.verified_bytes == length,
            &format!("independent full verification counters disagree: {progress:?}"),
        )?;
    }
    let archive_bytes =
        std::fs::read(root.join("archive-output").join(name)).map_err(|error| error.to_string())?;
    let later_bytes = std::fs::read(root.join("later-output/later-source.bin"))
        .map_err(|error| error.to_string())?;
    require(
        archive_bytes == expected,
        "archive Copy final data differs from actual native decompression",
    )?;
    require(
        later_bytes == vec![0x5a; 4096],
        "dependent native Copy final bytes differ",
    )?;
    Ok(())
}

async fn native_case(shared_pane: bool) {
    let temporary = NativeTempDir::new().expect("owned native temporary directory");
    let fixture_result = (|| -> Result<()> {
        std::fs::copy(fixture(), temporary.0.join("source.gz"))
            .map_err(|error| error.to_string())?;
        std::fs::create_dir(temporary.0.join("archive-output"))
            .map_err(|error| error.to_string())?;
        std::fs::create_dir(temporary.0.join("later-output")).map_err(|error| error.to_string())?;
        std::fs::write(temporary.0.join("later-source.bin"), vec![0x5a; 4096])
            .map_err(|error| error.to_string())?;
        Ok(())
    })();
    fixture_result.expect("copy protected media into independent owned native fixture");
    let close = Arc::new(CloseState::default());
    let read_hold = Arc::new(Hold::default());
    let release = ReleaseAll(vec![close.hold.clone(), read_hold.clone()]);
    let native_reads = Arc::new(AtomicU64::new(0));
    let provider = Arc::new(InstalledProvider::from_installation(installation(), None));
    let capture = held_capture(provider.clone(), close.clone());
    let mut session = Session::start(provider.clone(), capture, native_reads.clone());
    let observed = exercise(
        &mut session,
        &temporary.0,
        &close,
        &read_hold,
        &native_reads,
        shared_pane,
    )
    .await;
    // Ordinary failed observations always release both holds before shutdown and
    // assertions. RAII is only the unexpected-panic fallback.
    drop(release);
    let shutdown = session.stop().await;
    let dropped_provider = tokio::task::spawn_blocking(move || drop(provider)).await;
    let removed = if shutdown.is_ok() && dropped_provider.is_ok() {
        std::fs::remove_dir_all(&temporary.0).map_err(|error| error.to_string())
    } else {
        Err("retirement acknowledgement missing; temporary data retained".into())
    };
    assert!(shutdown.is_ok(), "{shutdown:?}");
    assert!(dropped_provider.is_ok(), "{dropped_provider:?}");
    assert!(removed.is_ok(), "{removed:?}");
    assert!(observed.is_ok(), "{observed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "protected installed native/archive workers, pristine restore 文档.gz, qualified native temp filesystem; run serially"]
async fn actual_archive_last_file_close_holds_copy_terminal_and_earlier_dependency() {
    native_case(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "protected installed native/archive workers, pristine restore 文档.gz, qualified native temp filesystem; run serially"]
async fn actual_archive_shared_pane_finishes_copy_without_waiting_global_mount_close() {
    native_case(true).await;
}
