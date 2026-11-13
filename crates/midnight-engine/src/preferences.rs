//! Bounded, serialized persistence of user preferences on backend workers.

use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{Event, FsError, PreferencesRevision, SettingsService, UserPreferences};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{Notify, watch},
    task::JoinSet,
};

/// One async worker, at most one native call and one latest pending save.
pub(crate) struct Controller {
    shared: Arc<Shared>,
    completion: watch::Receiver<bool>,
}

struct Shared {
    state: Mutex<State>,
    disk_io: Arc<midnight_work_budget::Gate>,
    work: Notify,
    stopping: Notify,
    complete: watch::Sender<bool>,
}

#[derive(Default)]
struct State {
    load_pending: bool,
    loading: bool,
    save_pending: Option<Save>,
    latest: Option<Save>,
    stopping: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Save {
    revision: PreferencesRevision,
    preferences: UserPreferences,
}

enum Work {
    Load,
    Save(Box<Save>),
    Wait,
    Stop,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        service: Arc<dyn SettingsService>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        Self::with_io(
            scope,
            service,
            outbox,
            tasks,
            midnight_work_budget::shared_io(),
        )
    }

    fn with_io(
        scope: TaskLifetime,
        service: Arc<dyn SettingsService>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
        disk_io: Arc<midnight_work_budget::Gate>,
    ) -> Self {
        let (complete, completion) = watch::channel(false);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            disk_io,
            work: Notify::new(),
            stopping: Notify::new(),
            complete,
        });
        let worker_shared = shared.clone();
        tasks.spawn(scope.future(run(scope.clone(), service, outbox, worker_shared)));
        Self { shared, completion }
    }

    /// Duplicate loads are coalesced while a load is pending or in progress.
    pub(crate) fn load(&self) -> bool {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopping {
            return false;
        }
        if state.loading {
            return true;
        }
        state.loading = true;
        state.load_pending = true;
        drop(state);
        self.shared.work.notify_one();
        true
    }

    /// True means accepted or an exact duplicate. A revision cannot be retargeted
    /// to different values; stale revisions cannot overwrite newer preferences.
    pub(crate) fn save(&self, revision: PreferencesRevision, preferences: UserPreferences) -> bool {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopping {
            return false;
        }
        let save = Save {
            revision,
            preferences,
        };
        if let Some(latest) = &state.latest {
            if revision < latest.revision {
                return false;
            }
            if revision == latest.revision {
                return latest == &save;
            }
        }
        state.latest = Some(save.clone());
        state.save_pending = Some(save);
        drop(state);
        self.shared.work.notify_one();
        true
    }

    /// Closing suppresses result delivery and pending loads. The worker still
    /// serially drains the latest pending save after its current native call.
    pub(crate) fn shutdown(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.stopping = true;
        state.load_pending = false;
        drop(state);
        self.shared.stopping.notify_one();
        self.shared.work.notify_one();
    }

    /// Await after shutdown with the application's bounded exit grace period.
    /// A blocked native call remains owned by its worker if that grace expires.
    pub(crate) async fn finished(&self) {
        let mut completion = self.completion.clone();
        loop {
            if *completion.borrow_and_update() {
                return;
            }
            if completion.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Shared {
    fn is_stopping(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopping
    }

    fn take_work(&self) -> Work {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(save) = state.save_pending.take() {
            return Work::Save(Box::new(save));
        }
        if state.stopping {
            return Work::Stop;
        }
        if state.load_pending {
            state.load_pending = false;
            return Work::Load;
        }
        Work::Wait
    }
}

async fn run(
    scope: TaskLifetime,
    service: Arc<dyn SettingsService>,
    outbox: Outbox,
    shared: Arc<Shared>,
) {
    let mut output_open = true;
    loop {
        let event = match shared.take_work() {
            Work::Load => {
                let service = service.clone();
                let disk_io = shared.disk_io.clone();
                let result = tokio::task::spawn_blocking(scope.blocking(move || {
                    let preferences = service.load_preferences()?;
                    preferences.validate()?;
                    let limit = preferences
                        .disk_io_concurrency
                        .effective_limit()
                        .map_err(disk_policy_error)?;
                    disk_io.set_limit(limit).map_err(disk_policy_error)?;
                    Ok(preferences)
                }))
                .await
                .unwrap_or_else(|_| Err(worker_error("load preferences")));
                shared
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .loading = false;
                Event::PreferencesLoaded(result.map(Box::new))
            }
            Work::Save(save) => {
                let save = *save;
                let revision = save.revision;
                let service = service.clone();
                let disk_io = shared.disk_io.clone();
                let result = tokio::task::spawn_blocking(scope.blocking(move || {
                    save.preferences.validate()?;
                    let limit = save
                        .preferences
                        .disk_io_concurrency
                        .effective_limit()
                        .map_err(disk_policy_error)?;
                    service.save_preferences(save.preferences)?;
                    disk_io.set_limit(limit).map_err(disk_policy_error)
                }))
                .await
                .unwrap_or_else(|_| Err(worker_error("save preferences")));
                Event::PreferencesSaved { revision, result }
            }
            Work::Wait => {
                shared.work.notified().await;
                continue;
            }
            Work::Stop => break,
        };
        if output_open && !shared.is_stopping() {
            output_open = deliver(&shared, &outbox, event).await;
        }
    }
    shared.complete.send_replace(true);
}

async fn deliver(shared: &Shared, outbox: &Outbox, event: Event) -> bool {
    // Reserve before taking the short state lock. Save/load notifications use a
    // separate slot and cannot interrupt or lose an acknowledgement.
    let permit = tokio::select! {
        result = outbox.sender.reserve() => match result {
            Ok(permit) => permit,
            Err(_) => return false,
        },
        _ = shared.stopping.notified() => return false,
    };
    let state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if state.stopping {
        return false;
    }
    // Serialize the final check and send with shutdown: even if both select
    // branches were ready, no result can be enqueued after closing is recorded.
    permit.send(event);
    drop(state);
    (outbox.wakeup)();
    true
}

fn worker_error(operation: &'static str) -> FsError {
    FsError::from_io(
        operation,
        Path::new(""),
        io::Error::other("The settings worker failed."),
    )
}

fn disk_policy_error(message: &'static str) -> FsError {
    FsError::from_io(
        "configure disk I/O concurrency",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}

#[cfg(test)]
#[path = "preferences/tests.rs"]
mod tests;
