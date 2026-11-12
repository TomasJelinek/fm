//! One independent native font enumeration with coalesced requests and a cache.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{Event, FsError, SettingsService};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{sync::Notify, task::JoinSet};

pub(crate) struct Controller {
    shared: Arc<Shared>,
}
struct Shared {
    state: Mutex<State>,
    work: Notify,
    stopping: Notify,
}
#[derive(Default)]
struct State {
    active: bool,
    stopping: bool,
    cached: Option<Vec<String>>,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        service: Option<Arc<dyn SettingsService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            work: Notify::new(),
            stopping: Notify::new(),
        });
        let worker = shared.clone();
        tasks.spawn(scope.future(run(scope.clone(), service, outbox, worker)));
        Self { shared }
    }
    pub(crate) fn list(&self) -> bool {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopping {
            return false;
        }
        if state.active {
            return true;
        }
        state.active = true;
        drop(state);
        self.shared.work.notify_one();
        true
    }
    pub(crate) fn shutdown(&self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stopping = true;
        self.shared.stopping.notify_one();
        self.shared.work.notify_one();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run(
    scope: TaskLifetime,
    service: Option<Arc<dyn SettingsService>>,
    outbox: Outbox,
    shared: Arc<Shared>,
) {
    loop {
        shared.work.notified().await;
        let cached = {
            let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopping {
                return;
            }
            state.cached.clone()
        };
        let result = match cached {
            Some(names) => Ok(names),
            None => {
                let service = service.clone();
                tokio::task::spawn_blocking(scope.blocking(move || {
                    service.map_or_else(|| Ok(Vec::new()), |service| service.font_families())
                }))
                .await
                .unwrap_or_else(|_| {
                    Err(FsError::from_io(
                        "enumerate fonts",
                        Path::new(""),
                        io::Error::other("The font worker failed."),
                    ))
                })
            }
        };
        {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopping {
                return;
            }
            if let Ok(names) = &result {
                state.cached = Some(names.clone());
            }
        }
        let permit = tokio::select! {
            permit = outbox.sender.reserve() => match permit { Ok(permit) => permit, Err(_) => return },
            _ = shared.stopping.notified() => return,
        };
        {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopping {
                return;
            }
            permit.send(Event::FontFamiliesLoaded(result));
            state.active = false;
        }
        (outbox.wakeup)();
    }
}

#[cfg(test)]
#[path = "fonts/tests.rs"]
mod tests;
