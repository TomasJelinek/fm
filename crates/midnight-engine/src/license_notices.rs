//! The About dialog's license notices on one backend worker. A newer request
//! replaces a queued one, `THIRD-PARTY.json` is parsed once, and no notice is
//! ever read on the UI thread.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{Event, LicenseNotice, LicenseNoticeRequest};
use midnight_platform_common::license_notices::{self, AuthorIndex, NoticeSources};
use std::sync::{Arc, Mutex};
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
    pending: Option<LicenseNoticeRequest>,
    stopping: bool,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        sources: NoticeSources,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            work: Notify::new(),
            stopping: Notify::new(),
        });
        let worker = shared.clone();
        tasks.spawn(scope.future(run(scope.clone(), Arc::new(sources), outbox, worker)));
        Self { shared }
    }

    /// Queue one request; an older request that has not started is dropped.
    pub(crate) fn request(&self, request: LicenseNoticeRequest) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopping {
            return;
        }
        state.pending = Some(request);
        drop(state);
        self.shared.work.notify_one();
    }

    pub(crate) fn shutdown(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.stopping = true;
        state.pending = None;
        drop(state);
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
    sources: Arc<NoticeSources>,
    outbox: Outbox,
    shared: Arc<Shared>,
) {
    let mut index: Option<Arc<AuthorIndex>> = None;
    loop {
        shared.work.notified().await;
        let request = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopping {
                return;
            }
            state.pending.take()
        };
        let Some(request) = request else { continue };
        let id = request.id;
        let (sources, cached) = (sources.clone(), index.clone());
        let read = tokio::task::spawn_blocking(scope.blocking(move || {
            let index =
                cached.unwrap_or_else(|| Arc::new(AuthorIndex::load(sources.inventory.as_deref())));
            let notice = license_notices::read_notice(&sources, &index, &request);
            (index, notice)
        }))
        .await;
        // A failed read still answers, so the dialog does not wait forever.
        let notice = match read {
            Ok((loaded, notice)) => {
                index = Some(loaded);
                notice
            }
            Err(_) => LicenseNotice {
                id,
                ..LicenseNotice::default()
            },
        };
        let permit = tokio::select! {
            permit = outbox.sender.reserve() => match permit { Ok(permit) => permit, Err(_) => return },
            _ = shared.stopping.notified() => return,
        };
        {
            let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopping {
                return;
            }
            permit.send(Event::LicenseNoticeReady(Box::new(notice)));
        }
        (outbox.wakeup)();
    }
}

#[cfg(test)]
#[path = "license_notices_tests.rs"]
mod tests;
