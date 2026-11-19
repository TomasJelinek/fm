//! Bounded backend orchestration. UI snapshots contain identities and source factories, never I/O.
use crate::{
    cache::{Cache, CachedFrame, DEFAULT_DISK_BYTES, DEFAULT_MEMORY_BYTES, Key, resolution_edge},
    decode::{self, Source},
    providers::{self, Provider},
    scheduler::{Item, Scheduler, Work},
};
use midnight_core::ThumbnailerAssociations;
use midnight_thumbnail_api::{Frame, Request};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// Invoked only by an admitted backend job, with that job's live cancellation flag.
pub type Capture = Arc<dyn Fn(&AtomicBool) -> io::Result<Arc<dyn Source>> + Send + Sync>;
/// One immutable revision belongs to a visible token, including all its animation samples.
fn pin_capture(capture: Capture) -> Capture {
    let source: Mutex<Option<Arc<dyn Source>>> = Mutex::new(None);
    Arc::new(move |cancel| {
        check_cancel(cancel)?;
        let mut pinned = source.lock().unwrap_or_else(|error| error.into_inner());
        check_cancel(cancel)?;
        if let Some(source) = &*pinned {
            return Ok(source.clone());
        }
        let source = capture(cancel)?;
        source.validate()?;
        check_cancel(cancel)?;
        let source: Arc<dyn Source> = Arc::new(FrozenSource {
            version: source.version(),
            length: source.length(),
            source,
        });
        *pinned = Some(source.clone());
        Ok(source)
    })
}
struct FrozenSource {
    source: Arc<dyn Source>,
    version: Vec<u8>,
    length: u64,
}
impl Source for FrozenSource {
    fn length(&self) -> u64 {
        self.length
    }
    fn version(&self) -> Vec<u8> {
        self.version.clone()
    }
    fn validate(&self) -> io::Result<()> {
        self.source.validate()?;
        if self.source.version() != self.version || self.source.length() != self.length {
            return Err(io::Error::other("Captured preview source revision changed"));
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = self.source.read_at(offset, length)?;
        self.validate()?;
        Ok(bytes)
    }
    fn read_at_cooperative(
        &self,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = self.source.read_at_cooperative(offset, length, cancelled)?;
        self.validate()?;
        Ok(bytes)
    }
    fn confirmed_local_ssd(&self) -> bool {
        self.source.confirmed_local_ssd()
    }
}
type Renderer = Arc<
    dyn Fn(&Target, &Provider, Work, &Mutex<Cache>, &AtomicBool) -> io::Result<CachedFrame>
        + Send
        + Sync,
>;
#[derive(Debug)]
struct CacheFailure(io::Error);
impl std::fmt::Display for CacheFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for CacheFailure {}
fn cache_failure(error: io::Error) -> io::Error {
    io::Error::other(CacheFailure(error))
}
#[derive(Clone)]
pub struct Target {
    pub token: u64,
    /// Requested physical edge, bounded by the existing worker protocol.
    pub edge: u32,
    pub identity: Vec<u8>,
    pub extension: String,
    pub selected: bool,
    pub hovered: bool,
    /// Only a positively established local-SSD hint admits the capture as shared I/O.
    /// Unknown, rotational, network and archive sources keep exclusive admission.
    pub confirmed_ssd: bool,
    pub capture: Capture,
}
pub struct View {
    /// Change generation when the source identities or accepted routing change.
    pub generation: u64,
    pub thumbnailer_associations: Arc<ThumbnailerAssociations>,
    pub targets: Vec<Target>,
}
pub struct Ready {
    pub generation: u64,
    pub token: u64,
    /// Decode coverage, independent of the source's actual pixel dimensions.
    pub edge: u32,
    pub frame: Arc<Frame>,
}
/// Invoked on a backend thread after a frame enters the bounded delivery queue, and after
/// a cache location or a notice is reported.
pub type ReadyNotifier = Arc<dyn Fn() + Send + Sync>;
struct Mailbox {
    next: Option<View>,
    retries: HashMap<(u64, u64, u8), (Work, u32)>,
    stopped: bool,
    clear_cache: bool,
    cache_location: Option<PathBuf>,
    notice: Option<String>,
    ready_notifier: Option<ReadyNotifier>,
    /// Bumped by every producer: requests, worker completions and freed delivery slots.
    /// The backend blocks only while it is unchanged, so an idle service does no work
    /// and no wakeup can be lost between its last inspection and its wait.
    signals: u64,
    /// Completions the backend has fully processed; lets tests order requests after them.
    #[cfg(test)]
    processed: u64,
}
struct Shared {
    mailbox: Mutex<Mailbox>,
    changed: Condvar,
}
impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            mailbox: Mutex::new(Mailbox {
                next: None,
                retries: HashMap::new(),
                stopped: false,
                clear_cache: false,
                cache_location: None,
                notice: None,
                ready_notifier: None,
                signals: 0,
                #[cfg(test)]
                processed: 0,
            }),
            changed: Condvar::new(),
        })
    }
    fn lock(&self) -> MutexGuard<'_, Mailbox> {
        self.mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    /// Publish a producer change and wake the backend.
    fn notify(&self, mut mailbox: MutexGuard<'_, Mailbox>) {
        mailbox.signals = mailbox.signals.wrapping_add(1);
        drop(mailbox);
        self.changed.notify_all();
    }
    fn signal(&self, update: impl FnOnce(&mut Mailbox)) {
        let mut mailbox = self.lock();
        update(&mut mailbox);
        self.notify(mailbox);
    }
    /// Store a cache location or notice for the presentation, then tell it: nothing
    /// else would make it poll again.
    fn report(&self, update: impl FnOnce(&mut Mailbox)) {
        let notifier = {
            let mut mailbox = self.lock();
            update(&mut mailbox);
            mailbox.ready_notifier.clone()
        };
        if let Some(notifier) = notifier {
            notifier();
        }
    }
    /// Block until a producer signals after `seen`, or until the optional deadline.
    fn wait(&self, seen: u64, deadline: Option<Instant>) {
        let mut mailbox = self.lock();
        while mailbox.signals == seen {
            mailbox = match deadline {
                None => self
                    .changed
                    .wait(mailbox)
                    .unwrap_or_else(|error| error.into_inner()),
                Some(deadline) => {
                    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                        return;
                    };
                    self.changed
                        .wait_timeout(mailbox, remaining)
                        .unwrap_or_else(|error| error.into_inner())
                        .0
                }
            };
        }
    }
    /// Providers and the cache inventory are needed only once a non-empty view or a cache
    /// clear arrives. Returns false when the service stops first.
    fn wait_for_demand(&self) -> bool {
        let mut mailbox = self.lock();
        loop {
            if mailbox.stopped {
                return false;
            }
            if mailbox.clear_cache
                || mailbox
                    .next
                    .as_ref()
                    .is_some_and(|view| !view.targets.is_empty())
            {
                return true;
            }
            mailbox = self
                .changed
                .wait(mailbox)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}
pub struct Service {
    shared: Arc<Shared>,
    ready: mpsc::Receiver<Ready>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Service {
    pub fn installed() -> Self {
        Self::initialize(|| {
            Ok((
                midnight_plugin_host::installed_executable().map_err(io::Error::other)?,
                default_cache_root()?,
            ))
        })
    }
    /// Initialization, discovery, cleanup and cache directory creation happen on the backend.
    pub fn start(executable: PathBuf, cache: PathBuf) -> Self {
        Self::initialize(move || Ok((executable, cache)))
    }
    fn initialize(
        locations: impl FnOnce() -> io::Result<(PathBuf, PathBuf)> + Send + 'static,
    ) -> Self {
        let shared = Shared::new();
        let (ready_tx, ready) = mpsc::sync_channel(4);
        let input = shared.clone();
        let thread = thread::spawn(move || {
            // Presentation enforces its 54 MiB pixel budget independently of resolution.
            // Four replies at the protocol's 512² RGBA bound and the 4 MiB warm cache
            // leave bounded headroom; source captures remain owned by admitted workers.
            let initialized = (|| {
                let (executable, cache) = locations()?;
                input.report(|mailbox| mailbox.cache_location = Some(cache.clone()));
                // Do not discover decoders or scan the cache inventory for a session that
                // never shows previews.
                if !input.wait_for_demand() {
                    return Ok(None);
                }
                let providers = providers::discover(&executable)?;
                let cache = Cache::open(&cache, DEFAULT_DISK_BYTES, DEFAULT_MEMORY_BYTES)?;
                Ok::<_, io::Error>(Some((providers, cache)))
            })();
            let (providers, cache) = match initialized {
                Ok(Some(value)) => value,
                Ok(None) => return,
                Err(error) => {
                    input.report(|mailbox| {
                        mailbox.notice = Some(format!("Previews are unavailable: {error}"));
                    });
                    return;
                }
            };
            run(
                input,
                ready_tx,
                providers,
                Arc::new(Mutex::new(cache)),
                Arc::new(render),
            );
        });
        Self {
            shared,
            ready,
            thread: Some(thread),
        }
    }
    /// Notify the presentation as soon as a frame, a cache location or a notice can be
    /// polled. The callback runs on a backend thread and must only schedule UI work; it
    /// runs once immediately when a location or notice arrived before it was installed.
    pub fn set_ready_notifier(&self, notifier: ReadyNotifier) {
        let reported = {
            let mut mailbox = self.shared.lock();
            mailbox.ready_notifier = Some(notifier.clone());
            mailbox.cache_location.is_some() || mailbox.notice.is_some()
        };
        if reported {
            notifier();
        }
    }
    pub fn set_view(&self, view: View) -> bool {
        if view.targets.len() > midnight_core::MAX_THUMBNAIL_VIEW_TARGETS
            || view
                .targets
                .iter()
                .any(|target| target.edge == 0 || target.edge > midnight_thumbnail_api::MAX_EDGE)
        {
            return false;
        }
        let mut mailbox = self.shared.lock();
        if mailbox.stopped {
            return false;
        }
        mailbox.next = Some(view);
        self.shared.notify(mailbox);
        true
    }
    pub fn poll(&self) -> Option<Ready> {
        let ready = self.ready.try_recv().ok();
        if ready.is_some() {
            // A delivery slot was freed; let a backend holding accepted frames continue.
            self.shared.signal(|_| {});
        }
        ready
    }
    /// Requeue a rejected upload only after presentation capacity or the view changes.
    /// No pixels are retained here; the worker reuses its validated cache entry.
    pub fn retry_delivery(&self, work: Work, retained_edge: u32) -> bool {
        if work.frame >= midnight_thumbnail_api::MAX_ANIMATION_FRAMES
            || work.edge == 0
            || work.edge > midnight_thumbnail_api::MAX_EDGE
            || retained_edge >= work.edge
        {
            return false;
        }
        let mut mailbox = self.shared.lock();
        let key = (work.generation, work.token, work.frame);
        if mailbox.stopped
            || (mailbox.retries.len()
                >= midnight_core::MAX_THUMBNAIL_VIEW_TARGETS
                    * usize::from(midnight_thumbnail_api::MAX_ANIMATION_FRAMES)
                && !mailbox.retries.contains_key(&key))
        {
            return false;
        }
        mailbox.retries.insert(key, (work, retained_edge));
        self.shared.notify(mailbox);
        true
    }
    pub fn clear_cache(&self) {
        self.shared.signal(|mailbox| mailbox.clear_cache = true);
    }
    /// Native cache location resolved by the backend initialization worker.
    pub fn take_cache_location(&self) -> Option<PathBuf> {
        self.shared.lock().cache_location.take()
    }
    pub fn take_notice(&self) -> Option<String> {
        self.shared.lock().notice.take()
    }
    pub fn stop(&self) {
        self.shared.signal(|mailbox| mailbox.stopped = true);
    }
}

/// Adapt already captured provider content without opening a native path.
/// The caller must supply only a positively established local SSD hint.
pub fn captured_source(
    file: midnight_plugin_host::CapturedFile,
    confirmed_ssd: bool,
) -> Arc<dyn Source> {
    Arc::new(CapturedSource {
        file,
        confirmed_ssd,
    })
}
struct CapturedSource {
    file: midnight_plugin_host::CapturedFile,
    confirmed_ssd: bool,
}
impl Source for CapturedSource {
    fn length(&self) -> u64 {
        self.file.length()
    }
    fn version(&self) -> Vec<u8> {
        self.file.stable_version()
    }
    fn validate(&self) -> io::Result<()> {
        self.file.revalidate().map_err(io::Error::other)
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.file.read_at(offset, length).map_err(io::Error::other)
    }
    fn confirmed_local_ssd(&self) -> bool {
        self.confirmed_ssd
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
        // Joining a native source call cannot run on the UI thread. The bounded service owns
        // its workers until their actual return and stops accepting all subsequent work.
        self.thread.take();
    }
}
struct Completion {
    work: Work,
    result: io::Result<CachedFrame>,
}
fn run(
    shared: Arc<Shared>,
    output: mpsc::SyncSender<Ready>,
    providers: Vec<Provider>,
    cache: Arc<Mutex<Cache>>,
    render: Renderer,
) {
    let mut scheduler = Scheduler::default();
    let mut targets: HashMap<u64, (Target, Provider)> = HashMap::new();
    let mut captures: HashMap<u64, Capture> = HashMap::new();
    let mut active: HashMap<u64, (Work, Arc<AtomicBool>, thread::JoinHandle<()>)> = HashMap::new();
    let (sender, receiver) = mpsc::sync_channel::<Completion>(2);
    // Accepted frames waiting for a free presentation slot. No new work starts while any
    // waits, so this holds at most the completions of already running jobs.
    let mut deliveries: VecDeque<Ready> = VecDeque::new();
    let mut stopped = false;
    let mut clearing = false;
    let mut generation = 0;
    let mut cache_error_reported = false;
    loop {
        let (seen, view, retries, notifier) = {
            let mut mailbox = shared.lock();
            stopped |= mailbox.stopped;
            clearing |= std::mem::take(&mut mailbox.clear_cache);
            (
                mailbox.signals,
                mailbox.next.take(),
                std::mem::take(&mut mailbox.retries),
                mailbox.ready_notifier.clone(),
            )
        };
        if let Some(view) = view {
            if generation != view.generation {
                captures.clear();
            }
            captures.retain(|token, _| view.targets.iter().any(|target| target.token == *token));
            // Frames of a previous generation or of rows that left the view are rejected by
            // presentation; the scheduler forgets their coverage too, so drop them here.
            deliveries.retain(|ready| {
                ready.generation == view.generation
                    && view
                        .targets
                        .iter()
                        .any(|target| target.token == ready.token)
            });
            generation = view.generation;
            targets.clear();
            let mut items = Vec::new();
            for mut target in view.targets {
                let Some(provider) = providers::select(
                    &providers,
                    &view.thumbnailer_associations,
                    &target.extension,
                    false,
                ) else {
                    continue;
                };
                target.capture = captures
                    .entry(target.token)
                    .or_insert_with(|| pin_capture(target.capture.clone()))
                    .clone();
                items.push(Item {
                    token: target.token,
                    edge: target.edge,
                    video: provider.manifest.video,
                    selected: target.selected,
                    hovered: target.hovered,
                });
                targets.insert(target.token, (target, provider.clone()));
            }
            scheduler.set_view(view.generation, items);
            for (work, cancel, _) in active.values() {
                if work.generation != view.generation
                    || targets.get(&work.token).is_none_or(|(target, _)| {
                        work.edge < target.edge
                            || (work.frame > 0 && !target.selected && !target.hovered)
                    })
                {
                    cancel.store(true, Ordering::Release);
                }
            }
        }
        for (_, (work, retained_edge)) in retries {
            scheduler.retry_delivery(work, retained_edge);
        }
        while let Ok(completion) = receiver.try_recv() {
            if !cache_error_reported
                && !clearing
                && !stopped
                && let Err(error) = &completion.result
                && error
                    .get_ref()
                    .is_some_and(|error| error.is::<CacheFailure>())
            {
                shared.report(|mailbox| {
                    mailbox.notice =
                        Some(format!("Could not store or read preview cache: {error}"));
                });
                cache_error_reported = true;
            }
            let cancelled =
                active
                    .remove(&completion.work.token)
                    .is_none_or(|(_, cancel, worker)| {
                        let _ = worker.join();
                        cancel.load(Ordering::Acquire)
                    });
            let accepted = if cancelled
                && completion
                    .result
                    .as_ref()
                    .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
            {
                // A changed view, clear or stop cancelled the job: no failure backoff.
                scheduler.abandon(completion.work);
                false
            } else {
                // An interruption nobody requested (e.g. a stopped archive mount) is a
                // failure; its backoff prevents an immediate decoder respawn loop.
                scheduler.complete_with_coverage(
                    completion.work,
                    completion.result.as_ref().ok().map(|frame| frame.edge),
                )
            };
            if accepted
                && !stopped
                && !clearing
                && let Ok(result) = completion.result
            {
                deliveries.push_back(Ready {
                    generation: completion.work.generation,
                    token: completion.work.token,
                    edge: result.edge,
                    frame: result.frame,
                });
            }
            #[cfg(test)]
            {
                shared.lock().processed += 1;
            }
        }
        if stopped || clearing {
            deliveries.clear();
        }
        // Deliver in completion order while presentation has capacity. A full queue no
        // longer blocks this loop: view changes and cancellations are still processed.
        while let Some(ready) = deliveries.pop_front() {
            match output.try_send(ready) {
                Ok(()) => {
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }
                Err(mpsc::TrySendError::Full(ready)) => {
                    deliveries.push_front(ready);
                    break;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    stopped = true;
                    deliveries.clear();
                }
            }
        }
        if stopped || clearing {
            for (_, cancel, _) in active.values() {
                cancel.store(true, Ordering::Release);
            }
        }
        if stopped && active.is_empty() {
            break;
        }
        if clearing && active.is_empty() {
            let result = cache
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            if let Err(error) = result {
                shared.report(|mailbox| {
                    mailbox.notice = Some(format!("Could not clear preview cache: {error}"));
                });
            }
            scheduler = Scheduler::default();
            scheduler.set_view(
                generation,
                targets
                    .values()
                    .map(|(target, provider)| Item {
                        token: target.token,
                        edge: target.edge,
                        video: provider.manifest.video,
                        selected: target.selected,
                        hovered: target.hovered,
                    })
                    .collect(),
            );
            clearing = false;
            cache_error_reported = false;
        }
        // Saturated presentation pauses new work until its frames have been taken.
        let can_start = !stopped && !clearing && deliveries.is_empty();
        if can_start {
            while let Some(work) = scheduler.next_work() {
                let Some((target, provider)) = targets.get(&work.token).cloned() else {
                    scheduler.complete(work, false);
                    continue;
                };
                let cancel = Arc::new(AtomicBool::new(false));
                let job_cancel = cancel.clone();
                let sender = sender.clone();
                let cache = cache.clone();
                let render = render.clone();
                let completed = shared.clone();
                let worker = thread::spawn(move || {
                    let result = render(&target, &provider, work, &cache, &job_cancel);
                    let _ = sender.send(Completion { work, result });
                    completed.signal(|_| {});
                });
                active.insert(work.token, (work, cancel, worker));
            }
        }
        // Sleep until a request, a completion or a freed delivery slot arrives; only a
        // failure backoff that can actually start new work sets a deadline.
        shared.wait(seen, can_start.then(|| scheduler.next_retry_at()).flatten());
    }
}
fn render(
    target: &Target,
    provider: &Provider,
    work: Work,
    cache: &Mutex<Cache>,
    cancel: &AtomicBool,
) -> io::Result<CachedFrame> {
    let source = {
        // A confirmed SSD capture shares the process-wide gate like the decoder's reads;
        // an unknown source keeps exclusive admission.
        let budget = midnight_work_budget::shared_io();
        let _permit = budget
            .acquire(
                midnight_work_budget::storage_admission(target.confirmed_ssd),
                || cancel.load(Ordering::Acquire),
            )
            .ok_or_else(|| io::Error::from(io::ErrorKind::Interrupted))?;
        (target.capture)(cancel)?
    };
    check_cancel(cancel)?;
    // Keep native paths or future authenticated provider addresses out of cache record plaintext.
    // The complete source revision remains in the key and is independently revalidated on every hit.
    let opaque_identity = blake3::hash(&target.identity);
    let edge = resolution_edge(work.edge);
    let key = Key::new(
        &provider.manifest.id,
        opaque_identity.as_bytes(),
        &*source,
        (edge, edge),
        &provider.manifest.version,
        work.frame,
    )?;
    if let Some(frame) = cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_covering(&key, &*source)
        .map_err(cache_failure)?
    {
        check_cancel(cancel)?;
        return Ok(frame);
    }
    if cancel.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let frame = decode::decode(
        &provider.executable,
        &*source,
        Request {
            id: work.token,
            source_length: source.length(),
            width: edge,
            height: edge,
            frame: work.frame,
        },
        cancel,
        Duration::from_secs(10),
    )?;
    if cancel.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .put(&key, &*source, frame)
        .map(|frame| CachedFrame { frame, edge })
        .map_err(cache_failure)
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

/// Resolving an environment setting belongs to backend setup; PrivateStore validates its ancestry.
pub fn default_cache_root() -> io::Result<PathBuf> {
    crate::paths::cache_root()
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "service_capture_tests.rs"]
mod capture_tests;

#[cfg(test)]
#[path = "service_archive_tests.rs"]
mod archive_tests;
