//! Coalescing worker validation keeps regex compilation out of UI callbacks: panel
//! filter drafts per pane, and Select/Deselect by mask patterns.
use crate::Outbox;
use midnight_core::{Event, FilterValidationRequest, FsError, MaskValidationRequest};
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
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        Self {
            filters: std::array::from_fn(|_| slot(&outbox, tasks)),
            masks: slot(&outbox, tasks),
        }
    }

    pub(crate) fn validate(&self, request: FilterValidationRequest) {
        offer(&self.filters[request.pane.index()], request);
    }

    pub(crate) fn validate_mask(&self, request: MaskValidationRequest) {
        offer(&self.masks, request);
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
