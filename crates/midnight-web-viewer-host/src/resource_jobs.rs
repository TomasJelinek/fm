//! One bounded resource worker per admitted browser session. The actor never owns native reads.
use crate::{
    Resources,
    api::{self, Resource, ResourceReply},
    resources::{ResourceCancellation, Store},
};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::{HashMap, VecDeque},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

const MAX_PENDING: usize = 64;

pub(crate) type CleanupStatus = Arc<Mutex<Option<String>>>;

fn remember(status: &CleanupStatus, error: String) {
    let mut status = status.lock().unwrap_or_else(|error| error.into_inner());
    if status.is_none() {
        *status = Some(error);
    }
}

fn checked_file(file: CapturedFile, status: &CleanupStatus) {
    match catch_unwind(AssertUnwindSafe(|| file.release_on_worker())) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => remember(status, error.to_string()),
        Err(panic) => {
            std::mem::forget(panic);
            remember(
                status,
                "HTML content cleanup panicked; ownership is unconfirmed".into(),
            );
        }
    }
}

/// Own before spawning either thread, so admission failures also use consuming backend release.
pub(crate) struct SourceOwner {
    main: Option<CapturedFile>,
    scope: Option<Arc<dyn Resources>>,
    status: CleanupStatus,
}

impl SourceOwner {
    pub(crate) fn new(
        main: CapturedFile,
        scope: Option<Arc<dyn Resources>>,
        status: CleanupStatus,
    ) -> Self {
        Self {
            main: Some(main),
            scope,
            status,
        }
    }

    fn finish(&mut self) {
        if let Some(main) = self.main.take() {
            checked_file(main, &self.status);
        }
        if let Some(scope) = self.scope.take() {
            match catch_unwind(AssertUnwindSafe(|| scope.release_on_worker())) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => remember(&self.status, error),
                Err(panic) => {
                    std::mem::forget(panic);
                    remember(
                        &self.status,
                        "HTML resource scope cleanup panicked; ownership is unconfirmed".into(),
                    );
                }
            }
        }
    }
}

impl Drop for SourceOwner {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Current {
    document: u64,
    cancellation: ResourceCancellation,
    resources_revoked: bool,
    revocation: Option<api::Event>,
    denial_staged: bool,
}

pub(crate) struct Generation {
    closed: Arc<AtomicBool>,
    current: Mutex<Current>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Default for Generation {
    fn default() -> Self {
        Self::with_closed(Arc::default())
    }
}

impl Generation {
    pub(crate) fn with_wake(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            wake,
            ..Self::default()
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn with_closed(closed: Arc<AtomicBool>) -> Self {
        Self {
            closed,
            wake: Arc::new(|| {}),
            current: Mutex::new(Current {
                document: 1,
                cancellation: ResourceCancellation::default(),
                resources_revoked: false,
                revocation: None,
                denial_staged: false,
            }),
        }
    }

    pub(crate) fn advance(&self, document: u64) -> Result<(), String> {
        let mut current = self
            .current
            .lock()
            .map_err(|_| "HTML generation lock failed")?;
        if self.closed.load(Ordering::Acquire)
            || current.resources_revoked
            || current.cancellation.is_cancelled()
            || document <= current.document
        {
            return Err("HTML generation was closed or exhausted".into());
        }
        current.cancellation.cancel();
        current.document = document;
        current.cancellation = ResourceCancellation::default();
        Ok(())
    }

    pub(crate) fn ensure_at_least(&self, document: u64) -> Result<(), String> {
        let mut current = self
            .current
            .lock()
            .map_err(|_| "HTML generation lock failed")?;
        if self.closed.load(Ordering::Acquire)
            || current.resources_revoked
            || current.cancellation.is_cancelled()
        {
            return Err("HTML generation was closed".into());
        }
        if document > current.document {
            current.cancellation.cancel();
            current.document = document;
            current.cancellation = ResourceCancellation::default();
        }
        Ok(())
    }

    fn token(&self, document: u64) -> Result<ResourceCancellation, String> {
        let current = self
            .current
            .lock()
            .map_err(|_| "HTML generation lock failed")?;
        if self.closed.load(Ordering::Acquire)
            || current.resources_revoked
            || current.cancellation.is_cancelled()
            || current.document != document
        {
            return Err("HTML generation was revoked".into());
        }
        Ok(current.cancellation.clone())
    }

    #[cfg(test)]
    pub(crate) fn resources_revoked(&self) -> bool {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .resources_revoked
    }

    pub(crate) fn revoke_resources(&self, origin: api::Event) -> bool {
        if origin_request(&origin).is_none() {
            return false;
        }
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.closed.load(Ordering::Acquire) || current.resources_revoked {
            return false;
        }
        current.resources_revoked = true;
        current.cancellation.cancel();
        // One actual bounded incoming resource event; no capture/provider ownership.
        current.revocation = Some(origin);
        drop(current);
        (self.wake)();
        true
    }

    #[cfg(test)]
    pub(crate) fn stage_denials(&self, document: u64, replies: &[ResourceReply]) {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Self::stage_locked(&mut current, document, replies);
    }

    fn stage_locked(current: &mut Current, document: u64, replies: &[ResourceReply]) {
        if let Some(origin) = &current.revocation
            && origin.header.document == document
            && let Some(request) = origin_request(origin)
            && replies.iter().any(|reply| matches!(reply, ResourceReply::Denied { request: denied, .. } if *denied == request))
        {
            current.denial_staged = true;
        }
    }

    pub(crate) fn take_revocation(&self, document: u64, request: u64) -> Option<api::Event> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.closed.load(Ordering::Acquire)
            && current.denial_staged
            && current.revocation.as_ref().is_some_and(|origin| {
                origin.header.document == document && origin_request(origin) == Some(request)
            })
        {
            return current.revocation.take();
        }
        None
    }

    pub(crate) fn close(&self) {
        let was_closed = self.closed.swap(true, Ordering::AcqRel);
        let current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        current.cancellation.cancel();
        drop(current);
        if !was_closed {
            (self.wake)();
        }
    }
}

fn origin_request(origin: &api::Event) -> Option<u64> {
    match &origin.kind {
        api::EventKind::ResourceResolve { request, .. }
        | api::EventKind::ResourceRead { request, .. } => Some(*request),
        _ => None,
    }
}

pub(crate) struct Reset {
    pub main: Resource,
    /// Must be sent under the OLD document before its replacement Open.
    pub denied: Vec<ResourceReply>,
}

enum JobKind {
    Resolve {
        request: u64,
        id: u64,
        base: u64,
        segments: Vec<String>,
    },
    Read {
        request: u64,
        resource: u64,
        offset: u64,
        length: u32,
    },
}

struct Job {
    document: u64,
    origin: api::Event,
    cancellation: ResourceCancellation,
    kind: JobKind,
}

impl Job {
    fn request(&self) -> u64 {
        match &self.kind {
            JobKind::Resolve { request, .. } | JobKind::Read { request, .. } => *request,
        }
    }
}

enum Done {
    Resolved {
        request: u64,
        base: u64,
        resource: Resource,
        segments: Vec<String>,
    },
    Reply(ResourceReply),
}

struct Completion {
    document: u64,
    counted: bool,
    done: Done,
}

struct ResetWork {
    document: u64,
    main: u64,
}

#[derive(Default)]
struct State {
    stopping: bool,
    reset: Option<ResetWork>,
    latest: Option<Job>,
    reads: VecDeque<Job>,
    releases: HashMap<u64, u64>,
    completed: VecDeque<Completion>,
    outstanding: usize,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    actor_wake: Arc<dyn Fn() + Send + Sync>,
}

enum Work {
    Reset(ResetWork),
    Release { document: u64, resource: u64 },
    Job(Job),
}

/// Actor-side capability metadata plus a fixed worker. No Store lock is held across I/O.
pub(crate) struct ResourceJobs {
    store: Store,
    shared: Arc<Shared>,
    generation: Arc<Generation>,
    document: u64,
    latest_navigation: bool,
    status: CleanupStatus,
    worker: Option<JoinHandle<()>>,
}

impl ResourceJobs {
    #[cfg(test)]
    pub(crate) fn new(owner: SourceOwner, generation: Arc<Generation>) -> Result<Self, String> {
        Self::with_wake(owner, generation, Arc::new(|| {}))
    }

    pub(crate) fn with_wake(
        owner: SourceOwner,
        generation: Arc<Generation>,
        actor_wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        let main = owner.main.as_ref().ok_or("Missing HTML source")?;
        let policy = owner
            .scope
            .as_ref()
            .map_or(crate::ResourcePathPolicy::default(), |scope| {
                scope.path_policy()
            });
        let ancestors = owner
            .scope
            .as_ref()
            .map_or(0, |scope| scope.ancestor_levels());
        let store = Store::new(main, policy, ancestors)?;
        let status = owner.status.clone();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            actor_wake,
        });
        let worker_shared = shared.clone();
        let worker_generation = generation.clone();
        let worker_status = status.clone();
        let worker = std::thread::Builder::new()
            .name("viewer-html-resources".into())
            .spawn(move || {
                let mut files = Files {
                    owner,
                    entries: HashMap::new(),
                    main: 0,
                    document: 0,
                    generated: policy == crate::ResourcePathPolicy::CanonicalRoot,
                };
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run(
                        &worker_shared,
                        &worker_generation,
                        &worker_status,
                        &mut files,
                    )
                }));
                if let Err(panic) = outcome {
                    std::mem::forget(panic);
                    remember(
                        &worker_status,
                        "HTML resource worker panicked; cleanup is unconfirmed".into(),
                    );
                    worker_generation.close();
                }
                // All actual captures remain owned by this worker through blocked calls and cleanup.
                files.finish();
                worker_shared.changed.notify_all();
                (worker_shared.actor_wake)();
            })
            .map_err(|error| format!("HTML resource worker startup failed: {error}"))?;
        Ok(Self {
            store,
            shared,
            generation,
            document: 1,
            latest_navigation: policy == crate::ResourcePathPolicy::CanonicalRoot,
            status,
            worker: Some(worker),
        })
    }

    pub(crate) fn reset(&mut self, document: u64, network: bool) -> Result<Reset, String> {
        let generation = self.generation.clone();
        let current = generation
            .current
            .lock()
            .map_err(|_| "HTML generation lock failed")?;
        if generation.closed.load(Ordering::Acquire)
            || current.resources_revoked
            || current.cancellation.is_cancelled()
            || document < self.document
        {
            return Err("HTML generation was revoked".into());
        }
        let denied = self.store.cancel_pending();
        let main = self.store.reset(network)?;
        self.document = document;
        drop(current);
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "HTML resource queue failed")?;
        let removed = state.latest.take().is_some() as usize
            + state.reads.len()
            + state
                .completed
                .iter()
                .filter(|completion| completion.counted)
                .count();
        state.reads.clear();
        state.completed.clear();
        state.releases.clear();
        state.outstanding -= removed;
        state.reset = Some(ResetWork {
            document,
            main: main.id,
        });
        drop(state);
        self.shared.changed.notify_all();
        Ok(Reset { main, denied })
    }

    #[cfg(test)]
    pub(crate) fn resolve(&mut self, request: u64, base: u64, url: &str) -> Option<ResourceReply> {
        self.resolve_with_header(
            api::Header {
                session: 1,
                document: self.document,
                sequence: 0,
            },
            request,
            base,
            url,
        )
    }

    pub(crate) fn resolve_with_header(
        &mut self,
        header: api::Header,
        request: u64,
        base: u64,
        url: &str,
    ) -> Option<ResourceReply> {
        if header.document != self.document {
            return Some(ResourceReply::Denied {
                request,
                reason: "HTML resource document was revoked".into(),
            });
        }
        // Transfer a queued target's reservation atomically with replacement. This lock covers
        // bounded metadata parsing only, never callbacks, captured I/O or cleanup.
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let replacing = if self.latest_navigation {
            state.latest.as_ref().map(Job::request)
        } else {
            None
        };
        let (id, segments) = match self.store.resolve(request, base, url, replacing) {
            Ok(value) => value,
            Err(reason) => return Some(ResourceReply::Denied { request, reason }),
        };
        let cancellation = match self.generation.token(self.document) {
            Ok(value) => value,
            Err(reason) => return Some(self.store.deny(request, &reason)),
        };
        if state.stopping
            || state.outstanding >= MAX_PENDING
                && (!self.latest_navigation || state.latest.is_none())
        {
            return Some(
                self.store
                    .deny(request, "HTML resource work budget exhausted"),
            );
        }
        let job = Job {
            document: self.document,
            origin: api::Event {
                header,
                kind: api::EventKind::ResourceResolve {
                    request,
                    base,
                    url: url.to_owned(),
                },
            },
            cancellation,
            kind: JobKind::Resolve {
                request,
                id,
                base,
                segments,
            },
        };
        let replaced = if self.latest_navigation {
            state.latest.replace(job)
        } else {
            state.reads.push_back(job);
            None
        };
        if replaced.is_none() {
            state.outstanding += 1;
        }
        drop(state);
        self.shared.changed.notify_all();
        replaced.map(|old| {
            self.store.deny(
                old.request(),
                "A newer HTML resource target replaced this request",
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn read(
        &mut self,
        request: u64,
        resource: u64,
        offset: u64,
        length: u32,
    ) -> Option<ResourceReply> {
        self.read_with_header(
            api::Header {
                session: 1,
                document: self.document,
                sequence: 0,
            },
            request,
            resource,
            offset,
            length,
        )
    }

    pub(crate) fn read_with_header(
        &mut self,
        header: api::Header,
        request: u64,
        resource: u64,
        offset: u64,
        length: u32,
    ) -> Option<ResourceReply> {
        if header.document != self.document {
            return Some(ResourceReply::Denied {
                request,
                reason: "HTML resource document was revoked".into(),
            });
        }
        if let Err(reason) = self.store.read(request, resource, offset, length) {
            return Some(ResourceReply::Denied { request, reason });
        }
        let cancellation = match self.generation.token(self.document) {
            Ok(value) => value,
            Err(reason) => return Some(self.store.deny(request, &reason)),
        };
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopping || state.outstanding >= MAX_PENDING {
            return Some(
                self.store
                    .deny(request, "HTML resource work budget exhausted"),
            );
        }
        state.outstanding += 1;
        state.reads.push_back(Job {
            document: self.document,
            origin: api::Event {
                header,
                kind: api::EventKind::ResourceRead {
                    request,
                    resource,
                    offset,
                    length,
                },
            },
            cancellation,
            kind: JobKind::Read {
                request,
                resource,
                offset,
                length,
            },
        });
        drop(state);
        self.shared.changed.notify_all();
        None
    }

    pub(crate) fn release(&mut self, resource: u64) -> Option<ResourceReply> {
        if !self.store.release(resource) {
            return Some(ResourceReply::Released { resource });
        }
        self.retire(resource);
        None
    }

    fn retire(&self, resource: u64) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Only actual grants or the at-most-one successful pending capture can enter this set.
        state.releases.insert(resource, self.document);
        drop(state);
        self.shared.changed.notify_all();
    }

    pub(crate) fn drain(&mut self) -> Result<Vec<ResourceReply>, String> {
        if let Some(error) = self
            .status
            .lock()
            .map_err(|_| "HTML resource status failed")?
            .clone()
        {
            return Err(error);
        }
        let completed = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| "HTML resource queue failed")?;
            let completed: Vec<_> = state.completed.drain(..).collect();
            state.outstanding -= completed
                .iter()
                .filter(|completion| completion.counted)
                .count();
            completed
        };
        self.shared.changed.notify_all();
        let mut replies = Vec::with_capacity(completed.len());
        for completion in completed {
            if let Some(reply) = self.accept_completion(completion) {
                replies.push(reply);
            }
        }
        let generation = self.generation.clone();
        let mut current = generation
            .current
            .lock()
            .map_err(|_| "HTML generation lock failed")?;
        if generation.closed.load(Ordering::Acquire)
            || current.resources_revoked
            || current.cancellation.is_cancelled()
            || current.document != self.document
        {
            let denied = self.store.cancel_pending();
            Generation::stage_locked(&mut current, self.document, &denied);
            replies.extend(denied);
        }
        Ok(replies)
    }

    /// The short metadata acceptance is the logical publication point. Replies
    /// admitted before revocation can still be in the actor's bounded wire batch.
    fn accept_completion(&mut self, completion: Completion) -> Option<ResourceReply> {
        let generation = self.generation.clone();
        let current = generation
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let live = !generation.current.is_poisoned()
            && !generation.closed.load(Ordering::Acquire)
            && !current.resources_revoked
            && !current.cancellation.is_cancelled()
            && current.document == self.document
            && completion.document == self.document;
        let mut retired = None;
        let reply = if live {
            Some(match completion.done {
                Done::Resolved {
                    request,
                    base,
                    resource,
                    segments,
                } => {
                    let id = resource.id;
                    match self
                        .store
                        .accept_resolved(request, base, resource, segments)
                    {
                        Ok(reply) => reply,
                        Err(reply) => {
                            retired = Some(id);
                            reply
                        }
                    }
                }
                Done::Reply(ResourceReply::Data {
                    request,
                    resource,
                    offset,
                    bytes,
                }) => self.store.accept_read(request, resource, offset, bytes),
                Done::Reply(ResourceReply::Denied { request, reason }) => {
                    self.store.deny(request, &reason)
                }
                Done::Reply(reply) => reply,
            })
        } else {
            if let Done::Resolved { resource, .. } = completion.done {
                retired = Some(resource.id);
            }
            // Pending IDs are settled once by drain, including completions that
            // raced cancellation after the worker's final liveness check.
            None
        };
        drop(current);
        if let Some(resource) = retired {
            self.retire(resource);
        }
        reply
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.stop()
    }

    fn stop(&mut self) -> Result<(), String> {
        self.generation.close();
        self.shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopping = true;
        self.shared.changed.notify_all();
        if let Some(worker) = self.worker.take()
            && let Err(panic) = worker.join()
        {
            std::mem::forget(panic);
            remember(&self.status, "HTML resource worker cleanup panicked".into());
        }
        match self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for ResourceJobs {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct Files {
    owner: SourceOwner,
    entries: HashMap<u64, CapturedFile>,
    main: u64,
    document: u64,
    generated: bool,
}

struct PendingFile {
    file: Option<CapturedFile>,
    status: CleanupStatus,
}
impl Drop for PendingFile {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            checked_file(file, &self.status);
        }
    }
}

impl Files {
    fn get(&self, resource: u64) -> Result<&CapturedFile, String> {
        if resource == self.main {
            self.owner.main.as_ref()
        } else {
            self.entries.get(&resource)
        }
        .ok_or_else(|| "Unknown or revoked HTML resource".into())
    }

    fn retire(&mut self, resource: u64) {
        if resource == self.main {
            self.main = 0;
        }
        if let Some(file) = self.entries.remove(&resource) {
            checked_file(file, &self.owner.status);
        }
    }

    fn reset(&mut self, reset: ResetWork) {
        for (_, file) in self.entries.drain() {
            checked_file(file, &self.owner.status);
        }
        self.main = reset.main;
        self.document = reset.document;
    }

    fn resolve(
        &mut self,
        cancellation: &ResourceCancellation,
        id: u64,
        base: u64,
        segments: &[String],
        generation: &Generation,
        origin: api::Event,
    ) -> Result<Resource, String> {
        if cancellation.is_cancelled() {
            return Err("HTML resource request cancelled".into());
        }
        if let Err(error) = self.get(base)?.revalidate() {
            self.invalidate(generation, origin);
            return Err(error.to_string());
        }
        let scope = self
            .owner
            .scope
            .as_ref()
            .ok_or("This source has no relative resource scope")?;
        let capture = scope.capture_with_cancellation(segments, cancellation);
        let (file, failure) = match capture {
            Ok(file) => (Some(file), None),
            Err(error) if !self.generated => return Err(error),
            Err(error) => (None, Some(error)),
        };
        let mut pending = PendingFile {
            file,
            status: self.owner.status.clone(),
        };
        if cancellation.is_cancelled() {
            return Err("HTML resource request cancelled".into());
        }
        if let Err(error) = self.get(base)?.revalidate() {
            self.invalidate(generation, origin);
            return Err(error.to_string());
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let file = pending.file.as_ref().ok_or("Missing HTML capture")?;
        if self.generated && file.length() > 1024 * 1024 {
            return Err("Generated HTML exceeds its 1 MiB resource limit".into());
        }
        if let Err(error) = file.revalidate() {
            self.invalidate(generation, origin);
            return Err(error.to_string());
        }
        let resource = Resource {
            id,
            length: file.length(),
            mime: crate::resources::mime_type(segments.last().map(String::as_str).unwrap_or(""))
                .into(),
        };
        self.entries
            .insert(id, pending.file.take().ok_or("Missing HTML capture")?);
        Ok(resource)
    }

    fn invalidate(&self, generation: &Generation, origin: api::Event) {
        if self.generated {
            generation.revoke_resources(origin);
        } else {
            generation.close();
        }
    }

    fn execute(&mut self, job: Job, generation: &Generation) -> Completion {
        let request = job.request();
        let document = job.document;
        let origin = job.origin;
        let done = if job.cancellation.is_cancelled() || document != self.document {
            Done::Reply(ResourceReply::Denied {
                request,
                reason: "HTML resource document was revoked".into(),
            })
        } else {
            match job.kind {
                JobKind::Resolve {
                    request,
                    id,
                    base,
                    segments,
                } => match self.resolve(&job.cancellation, id, base, &segments, generation, origin)
                {
                    Ok(resource) => Done::Resolved {
                        request,
                        base,
                        resource,
                        segments,
                    },
                    Err(reason) => Done::Reply(ResourceReply::Denied { request, reason }),
                },
                JobKind::Read {
                    request,
                    resource,
                    offset,
                    length,
                } => {
                    let result = self.get(resource).and_then(|file| {
                        // Derived previews validate their captured parent independently of
                        // browser/UI progress. Ordinary assets retain their existing read path.
                        if self.generated
                            && let Err(error) = file.revalidate()
                        {
                            self.invalidate(generation, origin.clone());
                            return Err(error.to_string());
                        }
                        let result = file
                            .read_at(offset, length)
                            .map_err(|error| error.to_string());
                        if self.generated
                            && let Err(error) = file.revalidate()
                        {
                            self.invalidate(generation, origin.clone());
                            return Err(error.to_string());
                        }
                        result
                    });
                    let reply = match result {
                        Ok(bytes) if !job.cancellation.is_cancelled() => ResourceReply::Data {
                            request,
                            resource,
                            offset,
                            bytes,
                        },
                        Ok(_) => ResourceReply::Denied {
                            request,
                            reason: "HTML resource request cancelled".into(),
                        },
                        Err(reason) => ResourceReply::Denied { request, reason },
                    };
                    Done::Reply(reply)
                }
            }
        };
        Completion {
            document,
            counted: true,
            done,
        }
    }

    fn finish(&mut self) {
        for (_, file) in self.entries.drain() {
            checked_file(file, &self.owner.status);
        }
        self.owner.finish();
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        self.finish();
    }
}

fn run(shared: &Shared, generation: &Generation, status: &CleanupStatus, files: &mut Files) {
    loop {
        let work = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            loop {
                if state.stopping {
                    return;
                }
                if let Some(reset) = state.reset.take() {
                    break Work::Reset(reset);
                }
                if let Some((&resource, &document)) = state.releases.iter().next() {
                    state.releases.remove(&resource);
                    break Work::Release { document, resource };
                }
                if let Some(job) = state.latest.take().or_else(|| state.reads.pop_front()) {
                    break Work::Job(job);
                }
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        let completion = match work {
            Work::Reset(reset) => {
                files.reset(reset);
                None
            }
            Work::Release { document, resource } => {
                files.retire(resource);
                Some(Completion {
                    document,
                    counted: false,
                    done: Done::Reply(ResourceReply::Released { resource }),
                })
            }
            Work::Job(job) => Some(files.execute(job, generation)),
        };
        if status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            generation.close();
            return;
        }
        if let Some(completion) = completion {
            let current = generation.token(completion.document).is_ok();
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !current || state.stopping {
                if completion.counted {
                    state.outstanding -= 1;
                }
                drop(state);
                if let Done::Resolved { resource, .. } = completion.done {
                    files.retire(resource.id);
                }
                (shared.actor_wake)();
                continue;
            }
            while state.completed.len() >= MAX_PENDING && !state.stopping {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.stopping {
                if completion.counted {
                    state.outstanding -= 1;
                }
                drop(state);
                if let Done::Resolved { resource, .. } = completion.done {
                    files.retire(resource.id);
                }
                continue;
            }
            state.completed.push_back(completion);
            drop(state);
            (shared.actor_wake)();
        }
    }
}

#[cfg(test)]
mod tests;
