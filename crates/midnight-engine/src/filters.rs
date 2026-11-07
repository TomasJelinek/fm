//! Coalescing worker validation keeps regex compilation out of UI callbacks.
use crate::Outbox;
use midnight_core::{Event, FilterValidationRequest, FsError};
use midnight_platform_common::filter::CompiledPanelFilter;
use std::{io, path::Path, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

#[derive(Clone, Default)]
struct Intent {
    id: u64,
    request: Option<Arc<FilterValidationRequest>>,
}

pub(crate) struct Controller {
    inputs: [watch::Sender<Intent>; 2],
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let inputs = std::array::from_fn(|_| {
            let (input, receiver) = watch::channel(Intent::default());
            tasks.spawn(run(outbox.clone(), receiver));
            input
        });
        Self { inputs }
    }

    pub(crate) fn validate(&self, request: FilterValidationRequest) {
        self.inputs[request.pane.index()].send_if_modified(|intent| {
            if request.id <= intent.id {
                return false;
            }
            intent.id = request.id;
            intent.request = Some(Arc::new(request));
            true
        });
    }
}

async fn run(outbox: Outbox, mut input: watch::Receiver<Intent>) {
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
        let filter = request.filter.clone();
        let (result, diagnostic) = match tokio::task::spawn_blocking(move || {
            CompiledPanelFilter::new(&filter).map(|_| ())
        })
        .await
        {
            Ok(Ok(())) => (Ok(()), None),
            Ok(Err(diagnostic)) => {
                let error = FsError::from_io(
                    "validate panel filter",
                    Path::new(""),
                    io::Error::new(io::ErrorKind::InvalidInput, diagnostic.message.clone()),
                );
                (Err(error), Some(diagnostic))
            }
            Err(error) => (
                Err(FsError::from_io(
                    "validate panel filter",
                    Path::new(""),
                    io::Error::other(error.to_string()),
                )),
                None,
            ),
        };
        let event = Event::PanelFilterValidated {
            request: (*request).clone(),
            result,
            diagnostic,
        };
        tokio::select! {
            biased;
            changed = input.changed() => { if changed.is_err() { return; } pending = true; }
            permit = outbox.sender.reserve() => {
                let Ok(permit) = permit else { return; };
                let sent = {
                    let latest = input.borrow();
                    if latest.id != request.id { false } else { permit.send(event); true }
                };
                if sent { (outbox.wakeup)(); }
            }
        }
    }
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;
