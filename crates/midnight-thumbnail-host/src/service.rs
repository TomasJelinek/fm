//! Bounded backend orchestration. UI snapshots contain identities and source factories, never I/O.
use crate::{
    cache::{Cache, DEFAULT_DISK_BYTES, Key},
    decode::{self, Source},
    providers::{self, Provider},
    scheduler::{Item, Scheduler, Work},
};
use midnight_thumbnail_api::{Frame, Request};
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub type Capture = Arc<dyn Fn() -> io::Result<Arc<dyn Source>> + Send + Sync>;
/// One immutable revision belongs to a visible token, including all its animation samples.
fn pin_capture(capture: Capture) -> Capture {
    let source: Mutex<Option<Arc<dyn Source>>> = Mutex::new(None);
    Arc::new(move || {
        let mut pinned = source.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(source) = &*pinned {
            return Ok(source.clone());
        }
        let source = capture()?;
        source.validate()?;
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
    fn confirmed_local_ssd(&self) -> bool {
        self.source.confirmed_local_ssd()
    }
}
type Renderer = Arc<
    dyn Fn(&Target, &Provider, Work, &Mutex<Cache>, &AtomicBool) -> io::Result<Arc<Frame>>
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
    pub identity: Vec<u8>,
    pub extension: String,
    pub selected: bool,
    pub hovered: bool,
    pub capture: Capture,
}
pub struct View {
    pub generation: u64,
    pub targets: Vec<Target>,
}
pub struct Ready {
    pub generation: u64,
    pub token: u64,
    pub frame: Arc<Frame>,
}
struct Mailbox {
    next: Option<View>,
    stopped: bool,
    clear_cache: bool,
    notice: Option<String>,
}
pub struct Service {
    mailbox: Arc<Mutex<Mailbox>>,
    ready: mpsc::Receiver<Ready>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Service {
    pub fn installed() -> Self {
        Self::initialize(|| Ok((std::env::current_exe()?, default_cache_root()?)))
    }
    /// Initialization, discovery, cleanup and cache directory creation happen on the backend.
    pub fn start(executable: PathBuf, cache: PathBuf) -> Self {
        Self::initialize(move || Ok((executable, cache)))
    }
    fn initialize(
        locations: impl FnOnce() -> io::Result<(PathBuf, PathBuf)> + Send + 'static,
    ) -> Self {
        let mailbox = Arc::new(Mutex::new(Mailbox {
            next: None,
            stopped: false,
            clear_cache: false,
            notice: None,
        }));
        let (ready_tx, ready) = mpsc::sync_channel(4);
        let input = mailbox.clone();
        let thread = thread::spawn(move || {
            // The presentation layer can retain 256 * 6 * 96² RGBA bytes (~54 MiB).
            // Leave room for bounded replies and this 4 MiB warm cache within 64 MiB.
            let initialized = (|| {
                let (executable, cache) = locations()?;
                let providers = providers::discover(&executable)?;
                let cache = Cache::open(&cache, DEFAULT_DISK_BYTES, 4 * 1024 * 1024)?;
                Ok::<_, io::Error>((providers, cache))
            })();
            let (providers, cache) = match initialized {
                Ok(value) => value,
                Err(error) => {
                    input
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .notice = Some(format!("Previews are unavailable: {error}"));
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
            mailbox,
            ready,
            thread: Some(thread),
        }
    }
    pub fn set_view(&self, view: View) -> bool {
        if view.targets.len() > 256 {
            return false;
        }
        self.mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .next = Some(view);
        true
    }
    pub fn poll(&self) -> Option<Ready> {
        self.ready.try_recv().ok()
    }
    pub fn clear_cache(&self) {
        self.mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear_cache = true;
    }
    pub fn take_notice(&self) -> Option<String> {
        self.mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .notice
            .take()
    }
    pub fn stop(&self) {
        self.mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped = true;
    }
}

/// Native source factory executes only when a bounded backend job is admitted. Virtual providers
/// can supply their own Capture closure with the same opaque identity and revision contract.
pub fn native_capture(path: PathBuf, confirmed_ssd: bool) -> Capture {
    Arc::new(move || {
        Ok(Arc::new(NativeSource {
            file: midnight_plugin_host::CapturedFile::capture(&path).map_err(io::Error::other)?,
            confirmed_ssd,
        }))
    })
}
struct NativeSource {
    file: midnight_plugin_host::CapturedFile,
    confirmed_ssd: bool,
}
impl Source for NativeSource {
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
    result: io::Result<Arc<Frame>>,
}
fn run(
    input: Arc<Mutex<Mailbox>>,
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
    let mut stopped = false;
    let mut clearing = false;
    let mut generation = 0;
    let mut cache_error_reported = false;
    loop {
        let view = {
            let mut mailbox = input.lock().unwrap_or_else(|error| error.into_inner());
            stopped |= mailbox.stopped;
            clearing |= std::mem::take(&mut mailbox.clear_cache);
            mailbox.next.take()
        };
        if let Some(view) = view {
            if generation != view.generation {
                captures.clear();
            }
            captures.retain(|token, _| view.targets.iter().any(|target| target.token == *token));
            generation = view.generation;
            targets.clear();
            let mut items = Vec::new();
            for mut target in view.targets {
                let Some(provider) = providers.iter().find(|provider| {
                    provider
                        .manifest
                        .extensions
                        .iter()
                        .any(|extension| extension.eq_ignore_ascii_case(&target.extension))
                }) else {
                    continue;
                };
                target.capture = captures
                    .entry(target.token)
                    .or_insert_with(|| pin_capture(target.capture.clone()))
                    .clone();
                items.push(Item {
                    token: target.token,
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
                        work.frame > 0 && !target.selected && !target.hovered
                    })
                {
                    cancel.store(true, Ordering::Release);
                }
            }
        }
        if stopped || clearing {
            for (_, cancel, _) in active.values() {
                cancel.store(true, Ordering::Release);
            }
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
                input
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .notice = Some(format!("Could not store or read preview cache: {error}"));
                cache_error_reported = true;
            }
            if let Some((_, _, worker)) = active.remove(&completion.work.token) {
                let _ = worker.join();
            }
            let accepted = if completion
                .result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
            {
                scheduler.abandon(completion.work);
                false
            } else {
                scheduler.complete(completion.work, completion.result.is_ok())
            };
            if accepted
                && !stopped
                && !clearing
                && let Ok(frame) = completion.result
            {
                let mut ready = Ready {
                    generation: completion.work.generation,
                    token: completion.work.token,
                    frame,
                };
                loop {
                    match output.try_send(ready) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            stopped = true;
                            break;
                        }
                        Err(mpsc::TrySendError::Full(value)) => ready = value,
                    }
                    let mailbox = input.lock().unwrap_or_else(|error| error.into_inner());
                    if mailbox.stopped {
                        stopped = true;
                        break;
                    }
                    if mailbox.clear_cache {
                        break;
                    }
                    drop(mailbox);
                    thread::sleep(Duration::from_millis(10));
                }
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
                input
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .notice = Some(format!("Could not clear preview cache: {error}"));
            }
            scheduler = Scheduler::default();
            scheduler.set_view(
                generation,
                targets
                    .values()
                    .map(|(target, provider)| Item {
                        token: target.token,
                        video: provider.manifest.video,
                        selected: target.selected,
                        hovered: target.hovered,
                    })
                    .collect(),
            );
            clearing = false;
            cache_error_reported = false;
        }
        if !stopped && !clearing {
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
                let worker = thread::spawn(move || {
                    let result = render(&target, &provider, work, &cache, &job_cancel);
                    let _ = sender.send(Completion { work, result });
                });
                active.insert(work.token, (work, cancel, worker));
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn render(
    target: &Target,
    provider: &Provider,
    work: Work,
    cache: &Mutex<Cache>,
    cancel: &AtomicBool,
) -> io::Result<Arc<Frame>> {
    let source = {
        let budget = midnight_work_budget::shared_io();
        let _permit = budget
            .acquire(midnight_work_budget::IO_SLOTS, || {
                cancel.load(Ordering::Acquire)
            })
            .ok_or_else(|| io::Error::from(io::ErrorKind::Interrupted))?;
        (target.capture)()?
    };
    // Keep native paths or future authenticated provider addresses out of cache record plaintext.
    // The complete source revision remains in the key and is independently revalidated on every hit.
    let opaque_identity = blake3::hash(&target.identity);
    let key = Key::new(
        &provider.manifest.id,
        opaque_identity.as_bytes(),
        &*source,
        (96, 96),
        &provider.manifest.version,
        work.frame,
    )?;
    if let Some(frame) = cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key, &*source)
        .map_err(cache_failure)?
    {
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
            width: 96,
            height: 96,
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
        .map_err(cache_failure)
}

/// Resolving an environment setting belongs to backend setup; PrivateStore validates its ancestry.
pub fn default_cache_root() -> io::Result<PathBuf> {
    crate::paths::cache_root()
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
