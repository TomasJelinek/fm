//! Three bounded coalescing lanes: draft validation and one paint job per pane.
use crate::Outbox;
use midnight_core::{Event, FilenameColorsRequest, FilenameColorsResult, FilenameColorsValidation};
use midnight_platform_common::filename_colors::CompiledFilenameColors;
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::watch, task::JoinSet};
#[cfg(test)]
mod tests;

#[derive(Clone)]
enum Work {
    Clear(u64),
    Validate(FilenameColorsValidation),
    Match(FilenameColorsRequest),
}

impl Work {
    fn id(&self) -> u64 {
        match self {
            Self::Clear(id) => *id,
            Self::Validate(request) => request.id,
            Self::Match(request) => request.id,
        }
    }
}

pub(crate) struct Controller {
    inputs: [watch::Sender<Option<Arc<Work>>>; 3],
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        Self {
            inputs: std::array::from_fn(|_| {
                let (input, receiver) = watch::channel(None);
                tasks.spawn(run(outbox.clone(), receiver));
                input
            }),
        }
    }
    pub(crate) fn validate(&self, request: FilenameColorsValidation) {
        self.submit(2, Work::Validate(request));
    }
    pub(crate) fn matches(&self, request: FilenameColorsRequest) {
        self.submit(request.pane.index(), Work::Match(request));
    }
    pub(crate) fn clear(&self, pane: midnight_core::PaneId, id: u64) {
        self.submit(pane.index(), Work::Clear(id));
    }
    fn submit(&self, lane: usize, work: Work) {
        self.inputs[lane].send_if_modified(|previous| {
            if previous.as_ref().is_some_and(|old| old.id() >= work.id()) {
                return false;
            }
            *previous = Some(Arc::new(work));
            true
        });
    }
}

fn compute(work: &Work, cancelled: &dyn Fn() -> bool) -> Option<Event> {
    match work {
        Work::Clear(_) => None,
        Work::Validate(request) => Some(Event::FilenameColorsValidated {
            request: request.clone(),
            result: CompiledFilenameColors::new(&request.rules).map(|_| ()),
        }),
        Work::Match(request) => {
            let result = CompiledFilenameColors::new(&request.rules).and_then(|compiled| {
                let mut styles = HashMap::new();
                let mut ids = std::collections::HashSet::new();
                for (index, entry) in request.source.iter().enumerate() {
                    if index.is_multiple_of(256) && cancelled() {
                        return Err("Cancelled".into());
                    }
                    if !ids.insert(entry.id) {
                        return Err("Ambiguous filename color identity".into());
                    }
                    if let Some(style) = compiled.style(&entry.name, entry.kind) {
                        styles.insert(entry.id, style);
                    }
                }
                Ok(Arc::new(FilenameColorsResult {
                    request: request.clone(),
                    styles,
                }))
            });
            (!cancelled()).then(|| Event::FilenameColorsReady {
                request: request.clone(),
                result,
            })
        }
    }
}

async fn run(outbox: Outbox, mut input: watch::Receiver<Option<Arc<Work>>>) {
    let mut pending = false;
    loop {
        if !pending && input.changed().await.is_err() {
            return;
        }
        pending = false;
        let Some(work) = input.borrow_and_update().clone() else {
            continue;
        };
        let worker = work.clone();
        let observer = input.clone();
        let result = tokio::task::spawn_blocking(move || {
            compute(&worker, &|| observer.has_changed().unwrap_or(true))
        })
        .await;
        let event = match result {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(error) => match work.as_ref() {
                Work::Clear(_) => continue,
                Work::Validate(request) => Event::FilenameColorsValidated {
                    request: request.clone(),
                    result: Err(error.to_string()),
                },
                Work::Match(request) => Event::FilenameColorsReady {
                    request: request.clone(),
                    result: Err(error.to_string()),
                },
            },
        };
        tokio::select! {
            biased;
            changed = input.changed() => { if changed.is_err() { return; } pending = true; }
            permit = outbox.sender.reserve() => {
                let Ok(permit) = permit else { return; };
                let latest = input.borrow();
                if latest.as_ref().is_some_and(|latest| latest.id() == work.id()) {
                    permit.send(event);
                    (outbox.wakeup)();
                }
            }
        }
    }
}
