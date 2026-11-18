use crate::resource_jobs::{Generation, SourceOwner};
use crate::{Provider, Resources, api};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};
#[path = "session_actor.rs"]
mod actor;

/// Immutable host ceiling. A renderer or injected command cannot raise this permission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PolicyCeiling {
    #[default]
    UserChoice,
    OfflineOnly,
}

#[derive(Default)]
struct Output {
    events: VecDeque<api::Event>,
    frame: Option<api::Event>,
}
impl Output {
    fn put_source_changed(&mut self, origin: api::Event) -> Result<(), String> {
        if self
            .events
            .len()
            .checked_add(2)
            .is_none_or(|length| length > 64)
        {
            return Err("Browser event queue exhausted".into());
        }
        let header = origin.header;
        self.put(origin)?;
        self.put(api::Event {
            header,
            kind: api::EventKind::Error {
                code: api::ErrorCode::ResourceChanged,
                message: "Source changed; showing previous contents. Reopen to refresh.".into(),
            },
        })
    }

    fn put(&mut self, event: api::Event) -> Result<(), String> {
        if matches!(event.kind, api::EventKind::Frame(_)) {
            self.frame = Some(event);
            return Ok(());
        }
        if matches!(event.kind, api::EventKind::State(_)) {
            self.events
                .retain(|old| !matches!(old.kind, api::EventKind::State(_)));
        }
        if self.events.len() >= 64 {
            return Err("Browser event queue exhausted".into());
        }
        self.events.push_back(event);
        Ok(())
    }
}

/// Bounded nonblocking command endpoint. Construction, destruction and drain belong on the
/// viewer backend; the frontend talks only to its existing ViewerSession queue.
pub struct Session {
    input: Arc<Mutex<midnight_web_viewer_sdk::CommandQueue>>,
    sequence: AtomicU64,
    output: Arc<Mutex<Output>>,
    closed: Arc<AtomicBool>,
    document: Arc<AtomicU64>,
    generation: Arc<Generation>,
    ceiling: PolicyCeiling,
    terminal: Arc<Mutex<Option<Result<(), String>>>>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn open(
        provider: Provider,
        source: CapturedFile,
        resources: Option<Arc<dyn Resources>>,
        viewport: api::Viewport,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, String> {
        Self::open_with_policy_ceiling(
            provider,
            source,
            resources,
            viewport,
            wake,
            PolicyCeiling::UserChoice,
        )
    }

    pub fn open_with_policy_ceiling(
        provider: Provider,
        source: CapturedFile,
        resources: Option<Arc<dyn Resources>>,
        viewport: api::Viewport,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        ceiling: PolicyCeiling,
    ) -> Result<Self, String> {
        let resources = resources.or_else(|| {
            source
                .resources()
                .map(|value| Arc::new(CapturedResources(value)) as Arc<dyn Resources>)
        });
        let cleanup_status = Arc::new(Mutex::new(None));
        // The guard exists before either thread is spawned. Failed admission still consumes the
        // source and resource capability through their checked backend release contracts.
        let owner = SourceOwner::new(source, resources, cleanup_status.clone());
        let input = Arc::new(Mutex::new(
            midnight_web_viewer_sdk::CommandQueue::new(64).map_err(|error| error.to_string())?,
        ));
        let receiver = input.clone();
        let output = Arc::new(Mutex::new(Output::default()));
        let closed = Arc::new(AtomicBool::new(false));
        let stopped = closed.clone();
        let document = Arc::new(AtomicU64::new(1));
        let generation = Arc::new(Generation::with_closed(closed.clone()));
        let requested_generation = generation.clone();
        let terminal = Arc::new(Mutex::new(None));
        let completion = terminal.clone();
        let requested_document = document.clone();
        let events = output.clone();
        let thread = std::thread::Builder::new()
            .name("viewer-html".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    actor::run(
                        provider,
                        owner,
                        ceiling,
                        viewport,
                        receiver,
                        &events,
                        &stopped,
                        &requested_document,
                        &requested_generation,
                        &wake,
                    )
                }))
                .unwrap_or_else(|panic| {
                    std::mem::forget(panic);
                    let message = "Browser actor panicked; cleanup is unconfirmed".to_owned();
                    *cleanup_status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(message.clone());
                    Err(message)
                });
                requested_generation.close();
                let cleanup = cleanup_status
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                let receipt = cleanup.map_or(Ok(()), Err);
                let result = receipt.clone().and(result);
                if let Err(message) = &result {
                    // Only diagnostics, never document contents or URLs, are sent to presentation.
                    if let Ok(mut output) = events.lock() {
                        output.frame = None;
                        output.events.clear();
                        let _ = output.put(api::Event {
                            header: api::Header {
                                session: 1,
                                document: 1,
                                sequence: 1,
                            },
                            kind: api::EventKind::Error {
                                code: api::ErrorCode::Runtime,
                                message: message.clone(),
                            },
                        });
                    }
                    wake();
                }
                // A renderer failure can still have confirmed capture cleanup. Do not retain
                // a native-source retirement credit just because the page failed to render.
                *completion.lock().unwrap_or_else(|error| error.into_inner()) = Some(receipt);
                stopped.store(true, Ordering::Release);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            input,
            sequence: AtomicU64::new(1),
            output,
            closed,
            document,
            generation,
            ceiling,
            terminal,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: api::CommandKind) -> bool {
        if self.closed.load(Ordering::Acquire)
            || matches!(
                command,
                api::CommandKind::Open { .. }
                    | api::CommandKind::ResourceReply(_)
                    | api::CommandKind::FrameAck { .. }
            )
        {
            return false;
        }
        if self.ceiling == PolicyCeiling::OfflineOnly
            && matches!(command, api::CommandKind::Policy(_))
        {
            return false;
        }
        // Serialize generation assignment with command admission, including concurrent callers.
        let Ok(mut queue) = self.input.lock() else {
            self.close();
            return false;
        };
        if matches!(command, api::CommandKind::Policy(_)) {
            let Some(next) = self.document.load(Ordering::Acquire).checked_add(1) else {
                self.close();
                return false;
            };
            if self.generation.advance(next).is_err() {
                self.close();
                return false;
            }
            self.document.store(next, Ordering::Release);
            if let Ok(mut output) = self.output.lock() {
                output.frame = None;
                output.events.clear();
            }
        }
        let sequence = self
            .sequence
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            });
        let accepted = sequence.is_ok_and(|sequence| {
            queue
                .push(api::Command {
                    header: api::Header {
                        session: 1,
                        document: self.document.load(Ordering::Acquire),
                        sequence,
                    },
                    kind: command,
                })
                .is_ok()
        });
        if !accepted {
            // Never lose ordered button/key releases and leave a page with stuck input.
            self.close();
            return false;
        }
        true
    }

    pub fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let Ok(mut output) = self.output.lock() else {
            return Vec::new();
        };
        let document = self.document.load(Ordering::Acquire);
        output.events.retain(|event| {
            event.header.document == document || matches!(event.kind, api::EventKind::Error { .. })
        });
        if output
            .frame
            .as_ref()
            .is_some_and(|frame| frame.header.document != document)
        {
            output.frame = None;
        }
        let count = maximum.min(output.events.len());
        let mut events: Vec<_> = output.events.drain(..count).collect();
        if events.len() < maximum
            && let Some(frame) = output.frame.take()
        {
            events.push(frame);
        }
        events
    }
    pub fn close(&self) {
        self.generation.close();
    }
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn policy_change_allowed(&self) -> bool {
        self.ceiling == PolicyCeiling::UserChoice
    }

    /// Consume on the viewer backend and qualify the actor and all captured resource cleanup.
    pub fn release_on_worker(mut self) -> Result<(), String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        self.close();
        if let Some(thread) = self.thread.take()
            && let Err(panic) = thread.join()
        {
            std::mem::forget(panic);
            return Err("Browser actor cleanup panicked".into());
        }
        self.terminal
            .lock()
            .map_err(|_| "Browser completion unavailable")?
            .take()
            .unwrap_or(Ok(()))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[derive(Debug)]
struct CapturedResources(Arc<dyn midnight_plugin_host::ContentResources>);
impl Resources for CapturedResources {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String> {
        self.0.capture(segments).map_err(|error| error.to_string())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        match Arc::try_unwrap(self) {
            Ok(CapturedResources(resources)) => resources
                .release_on_worker()
                .map_err(|error| error.to_string()),
            // Another wrapper reference still owns the underlying capability.
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[path = "session_policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn event(sequence: u64, kind: api::EventKind) -> api::Event {
        api::Event {
            header: api::Header {
                session: 1,
                document: 1,
                sequence,
            },
            kind,
        }
    }
    #[test]
    fn frame_mailbox_retains_only_latest_owned_pixels() {
        let mut output = Output::default();
        for sequence in 1..=300 {
            output
                .put(event(
                    sequence,
                    api::EventKind::Frame(api::Frame {
                        viewport_revision: 1,
                        acknowledged_input: 1,
                        width: 1,
                        height: 1,
                        rgba: vec![0; 4],
                    }),
                ))
                .unwrap();
        }
        assert_eq!(output.frame.as_ref().unwrap().header.sequence, 300);
        assert!(output.events.is_empty());
    }
    #[test]
    fn control_events_are_bounded_and_not_silently_dropped() {
        let mut output = Output::default();
        for sequence in 1..=64 {
            output.put(event(sequence, api::EventKind::Ready)).unwrap();
        }
        assert!(output.put(event(65, api::EventKind::Closed)).is_err());
        assert!(
            output
                .put(event(
                    66,
                    api::EventKind::State(api::State {
                        title: String::new(),
                        url: String::new(),
                        loading: false,
                        editable: false,
                        zoom: 1.0,
                    }),
                ))
                .is_err()
        );
        assert_eq!(output.events.len(), 64);
    }
}
