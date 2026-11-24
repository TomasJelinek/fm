//! Coalescing worker validation keeps regex compilation out of UI callbacks: panel
//! filter drafts per pane, and Select/Deselect by mask patterns.
use crate::Outbox;
use midnight_core::{
    Cancellation, Event, FilterValidationRequest, FindFeedBuildRequest, FindFeedBuildResult,
    FindFeedBuildStamp, FindFeedProjectionRequest, FindFeedProjectionResult,
    FindFeedProjectionStamp, FsError, LocaleService, MaskValidationRequest,
};
use midnight_platform_common::filter::{CompiledPanelFilter, validate_selection_mask};
use std::{io, path::Path, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

/// A request whose answer one worker computes. Each kind keeps its own slots, so
/// one kind's requests never supersede another's.
trait Validation: Send + Sync + 'static {
    fn id(&self) -> u64;
    /// On a blocking worker: compile and answer.
    fn answer(&self) -> Event;
    /// The answer when the blocking worker failed.
    fn failed(&self, message: String) -> Event;
    fn superseded(&self) {}
}

struct FeedProjectionWork {
    request: std::sync::Mutex<Option<Arc<FindFeedProjectionRequest>>>,
    stamp: FindFeedProjectionStamp,
    cancel: Cancellation,
    locale: Option<Arc<dyn LocaleService>>,
}

impl Validation for FeedProjectionWork {
    fn id(&self) -> u64 {
        self.stamp.id
    }
    fn answer(&self) -> Event {
        let request = match self.request.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => None,
        };
        let Some(request) = request else {
            return self.failed("Search result request is unavailable.".into());
        };
        Event::FindFeedProjected(Box::new(FindFeedProjectionResult {
            request: self.stamp,
            result: crate::feed_projection::project(&request, self.locale.as_deref()),
        }))
    }
    fn failed(&self, message: String) -> Event {
        Event::FindFeedProjected(Box::new(FindFeedProjectionResult {
            request: self.stamp,
            result: Err(FsError::from_io(
                "project search results",
                Path::new(""),
                io::Error::other(message),
            )),
        }))
    }
    fn superseded(&self) {
        self.cancel.cancel();
    }
}

struct FeedBuildWork {
    request: std::sync::Mutex<Option<Arc<FindFeedBuildRequest>>>,
    stamp: FindFeedBuildStamp,
    cancel: Cancellation,
}
impl Validation for FeedBuildWork {
    fn id(&self) -> u64 {
        self.stamp.id
    }
    fn answer(&self) -> Event {
        let request = match self.request.lock() {
            Ok(mut request) => request.take(),
            Err(_) => None,
        };
        let Some(request) = request else {
            return self.failed("Feed preparation request is unavailable.".into());
        };
        Event::FindFeedBuilt(Box::new(FindFeedBuildResult {
            request: self.stamp,
            result: request.prepare(),
        }))
    }
    fn failed(&self, message: String) -> Event {
        Event::FindFeedBuilt(Box::new(FindFeedBuildResult {
            request: self.stamp,
            result: Err(FsError::from_io(
                "feed search results",
                Path::new(""),
                io::Error::other(message),
            )),
        }))
    }
    fn superseded(&self) {
        self.cancel.cancel();
    }
}
// Build and later projection share one bounded slot per pane. Completed work
// retains only this small stamp/cancellation owner, never its input result array.
enum FeedWork {
    Projection(FeedProjectionWork),
    Build(FeedBuildWork),
}
impl Validation for FeedWork {
    fn id(&self) -> u64 {
        match self {
            Self::Projection(work) => work.id(),
            Self::Build(work) => work.id(),
        }
    }
    fn answer(&self) -> Event {
        match self {
            Self::Projection(work) => work.answer(),
            Self::Build(work) => work.answer(),
        }
    }
    fn failed(&self, message: String) -> Event {
        match self {
            Self::Projection(work) => work.failed(message),
            Self::Build(work) => work.failed(message),
        }
    }
    fn superseded(&self) {
        match self {
            Self::Projection(work) => work.superseded(),
            Self::Build(work) => work.superseded(),
        }
    }
}

impl Validation for FilterValidationRequest {
    fn id(&self) -> u64 {
        self.id
    }

    fn answer(&self) -> Event {
        let (result, diagnostic) = match CompiledPanelFilter::new(&self.filter) {
            Ok(_) => (Ok(()), None),
            Err(diagnostic) => {
                let error = FsError::from_io(
                    "validate panel filter",
                    Path::new(""),
                    io::Error::new(io::ErrorKind::InvalidInput, diagnostic.message.clone()),
                );
                (Err(error), Some(diagnostic))
            }
        };
        Event::PanelFilterValidated {
            request: self.clone(),
            result,
            diagnostic,
        }
    }

    fn failed(&self, message: String) -> Event {
        Event::PanelFilterValidated {
            request: self.clone(),
            result: Err(FsError::from_io(
                "validate panel filter",
                Path::new(""),
                io::Error::other(message),
            )),
            diagnostic: None,
        }
    }
}

impl Validation for MaskValidationRequest {
    fn id(&self) -> u64 {
        self.id
    }

    fn answer(&self) -> Event {
        Event::SelectionMaskValidated {
            request: self.clone(),
            result: validate_selection_mask(&self.pattern),
        }
    }

    fn failed(&self, message: String) -> Event {
        Event::SelectionMaskValidated {
            request: self.clone(),
            result: Err(format!("The mask could not be checked: {message}")),
        }
    }
}

struct Intent<R> {
    id: u64,
    request: Option<Arc<R>>,
}

impl<R> Clone for Intent<R> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            request: self.request.clone(),
        }
    }
}

pub(crate) struct Controller {
    filters: [watch::Sender<Intent<FilterValidationRequest>>; 2],
    /// One Select/Deselect by mask dialog exists at a time.
    masks: watch::Sender<Intent<MaskValidationRequest>>,
    feeds: [watch::Sender<Intent<FeedWork>>; 2],
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        Self {
            filters: std::array::from_fn(|_| slot(&outbox, tasks)),
            masks: slot(&outbox, tasks),
            feeds: std::array::from_fn(|_| slot(&outbox, tasks)),
        }
    }

    pub(crate) fn validate(&self, request: FilterValidationRequest) {
        offer(&self.filters[request.pane.index()], request);
    }

    pub(crate) fn validate_mask(&self, request: MaskValidationRequest) {
        offer(&self.masks, request);
    }
    pub(crate) fn project_feed(
        &self,
        request: Arc<FindFeedProjectionRequest>,
        locale: Option<Arc<dyn LocaleService>>,
    ) {
        let stamp = FindFeedProjectionStamp {
            id: request.id,
            pane: request.pane,
            rows_serial: request.rows_serial,
            selection_revision: request.selection_revision,
        };
        let cancel = request.cancel.clone();
        offer(
            &self.feeds[request.pane.index()],
            FeedWork::Projection(FeedProjectionWork {
                request: std::sync::Mutex::new(Some(request)),
                stamp,
                cancel,
                locale,
            }),
        );
    }
    pub(crate) fn build_feed(&self, request: Arc<FindFeedBuildRequest>) {
        let stamp = request.stamp();
        let cancel = request.cancel.clone();
        offer(
            &self.feeds[request.pane.index()],
            FeedWork::Build(FeedBuildWork {
                request: std::sync::Mutex::new(Some(request)),
                stamp,
                cancel,
            }),
        );
    }
}

fn slot<R: Validation>(outbox: &Outbox, tasks: &mut JoinSet<()>) -> watch::Sender<Intent<R>> {
    let (input, receiver) = watch::channel(Intent {
        id: 0,
        request: None,
    });
    tasks.spawn(run(outbox.clone(), receiver));
    input
}

/// Only a newer request replaces the slot's intent; an older one is dropped.
fn offer<R: Validation>(input: &watch::Sender<Intent<R>>, request: R) {
    input.send_if_modified(|intent| {
        if request.id() <= intent.id {
            return false;
        }
        if let Some(old) = &intent.request {
            old.superseded();
        }
        intent.id = request.id();
        intent.request = Some(Arc::new(request));
        true
    });
}

async fn run<R: Validation>(outbox: Outbox, mut input: watch::Receiver<Intent<R>>) {
    let mut pending = false;
    loop {
        if !pending && input.changed().await.is_err() {
            return;
        }
        pending = false;
        let intent = input.borrow_and_update().clone();
        let Some(request) = intent.request else {
            continue;
        };
        let worker = request.clone();
        let event = match tokio::task::spawn_blocking(move || worker.answer()).await {
            Ok(event) => event,
            Err(error) => request.failed(error.to_string()),
        };
        tokio::select! {
            biased;
            changed = input.changed() => { if changed.is_err() { return; } pending = true; }
            permit = outbox.sender.reserve() => {
                let Ok(permit) = permit else { return; };
                let sent = {
                    let latest = input.borrow();
                    if latest.id != request.id() { false } else { permit.send(event); true }
                };
                if sent { (outbox.wakeup)(); }
            }
        }
    }
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;

#[cfg(test)]
mod feed_build_tests {
    use super::*;
    use midnight_core::*;
    fn input(id: u64) -> Arc<FindFeedBuildRequest> {
        Arc::new(FindFeedBuildRequest {
            id,
            pane: PaneId::Left,
            session: FindSessionId(id),
            generation: FindGeneration(1),
            results: vec![Arc::new(FindResult {
                id: FindResultId(1),
                entry: FindEntry {
                    path: std::env::temp_dir().join(format!("feed-build-{id}.txt")),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [0; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
                duplicate_group: None,
                duplicate_group_size: 0,
                archive: None,
            })]
            .into(),
            order: None,
            selection_bits: Arc::from([]),
            explicit_marks: false,
            cursor: Some(FindResultId(1)),
            cancel: Cancellation::default(),
        })
    }
    #[tokio::test]
    async fn newest_feed_build_wins_and_idle_slot_releases_captured_find_results() {
        let (sender, mut events) = tokio::sync::mpsc::channel(1);
        sender.send(Event::VolumesUpdated(vec![])).await.unwrap();
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let first = input(1);
        let cancel = first.cancel.clone();
        controller.build_feed(first);
        tokio::task::yield_now().await;
        let latest = input(2);
        let retained = Arc::downgrade(&latest.results);
        controller.build_feed(latest);
        assert!(matches!(
            events.recv().await,
            Some(Event::VolumesUpdated(_))
        ));
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        let Event::FindFeedBuilt(receipt) = event else {
            panic!("latest build receipt");
        };
        assert_eq!(receipt.request.id, 2);
        assert_eq!(receipt.request.session, FindSessionId(2));
        assert!(cancel.is_cancelled());
        assert!(receipt.result.is_ok());
        assert!(
            retained.upgrade().is_none(),
            "the slot retains no original Find result pointers once work finished"
        );
        drop(receipt);
        drop(controller);
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    }
}
