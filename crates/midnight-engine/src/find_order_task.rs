//! One coalescing sort task per fixed Find slot; never reorder validation authority.
use super::*;
use crate::find_sort::KeyCache;
use std::time::Instant;

pub(super) async fn run(
    scope: TaskLifetime,
    state: Arc<Mutex<State>>,
    locale: Option<Arc<dyn LocaleService>>,
    outbox: Outbox,
    mut input: watch::Receiver<u64>,
    lifetime: Cancellation,
) {
    let mut last = Instant::now() - Duration::from_millis(250);
    // Locale keys of the current generation survive across its re-sorts.
    let mut keys: Option<((FindSessionId, FindGeneration), KeyCache)> = None;
    while input.changed().await.is_ok() {
        if lifetime.is_cancelled() {
            return;
        }
        if let Some(delay) = Duration::from_millis(250).checked_sub(last.elapsed()) {
            tokio::time::sleep(delay).await;
        }
        input.borrow_and_update();
        if lifetime.is_cancelled() {
            return;
        }
        let snapshot = {
            let state = state.lock().unwrap_or_else(|error| error.into_inner());
            state.owner.map(|owner| {
                (
                    owner,
                    state.sort_revision,
                    state.sort,
                    state.results.clone(),
                    state.order_cancel.clone(),
                )
            })
        };
        let Some((owner, revision, sort, results, cancel)) = snapshot else {
            // Closed: release the previous generation's keys on a worker.
            if let Some(retired) = keys.take() {
                let _ = tokio::task::spawn_blocking(scope.blocking(move || drop(retired))).await;
            }
            continue;
        };
        let previous = keys.take();
        let count = results.len();
        let worker_state = state.clone();
        let locale = locale.clone();
        let outbox = outbox.clone();
        last = Instant::now();
        let returned = tokio::task::spawn_blocking(scope.blocking(move || {
            // Another generation's keys are released here, on the worker.
            let mut cache = match previous {
                Some((current, cache)) if current == owner => cache,
                _ => KeyCache::default(),
            };
            let update = match crate::find_sort::order(
                &results,
                sort,
                revision,
                locale.as_deref(),
                &cancel,
                &mut cache,
            ) {
                Ok(order) => Some(FindUpdate::Sorted(Arc::new(order))),
                Err(error) if !cancel.is_cancelled() => {
                    Some(FindUpdate::SortFailed { revision, error })
                }
                Err(_) => None,
            };
            if let Some(update) = update {
                deliver(
                    &outbox,
                    &worker_state,
                    owner,
                    revision,
                    count,
                    update,
                    &cancel,
                );
            }
            // The result snapshot is released here, on the worker.
            drop(results);
            cache
        }))
        .await;
        keys = returned.ok().map(|cache| (owner, cache));
    }
}

/// Publish only while the generation, sort revision and result count still
/// match, checking and sending under the same lock.
fn deliver(
    outbox: &Outbox,
    state: &Mutex<State>,
    owner: (FindSessionId, FindGeneration),
    revision: u64,
    count: usize,
    update: FindUpdate,
    cancel: &Cancellation,
) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let current = state.lock().unwrap_or_else(|error| error.into_inner());
        if current.owner != Some(owner)
            || current.sort_revision != revision
            || current.results.len() != count
        {
            return;
        }
        match outbox.sender.try_reserve() {
            Ok(permit) => {
                permit.send(Event::Find(Box::new(FindEvent {
                    session: owner.0,
                    generation: owner.1,
                    update,
                })));
                drop(current);
                (outbox.wakeup)();
                return;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
            Err(mpsc::error::TrySendError::Full(_)) => {}
        }
        drop(current);
        std::thread::park_timeout(Duration::from_millis(10));
    }
}
