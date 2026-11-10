//! One coalescing sort task per fixed Find slot; never reorder validation authority.
use super::*;
use std::time::Instant;

pub(super) async fn run(
    state: Arc<Mutex<State>>,
    locale: Option<Arc<dyn LocaleService>>,
    outbox: Outbox,
    mut input: watch::Receiver<u64>,
    lifetime: Cancellation,
) {
    let mut last = Instant::now() - Duration::from_millis(250);
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
            continue;
        };
        let count = results.len();
        let worker_state = state.clone();
        let locale = locale.clone();
        let outbox = outbox.clone();
        last = Instant::now();
        let _ = tokio::task::spawn_blocking(move || {
            let update =
                match crate::find_sort::order(&results, sort, revision, locale.as_deref(), &cancel)
                {
                    Ok(order) => FindUpdate::Sorted(Arc::new(order)),
                    Err(error) if !cancel.is_cancelled() => {
                        FindUpdate::SortFailed { revision, error }
                    }
                    Err(_) => return,
                };
            loop {
                if cancel.is_cancelled() {
                    return;
                }
                let current = worker_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if current.owner != Some(owner)
                    || current.sort_revision != revision
                    || current.results.len() != count
                {
                    return;
                }
                match outbox.sender.try_reserve() {
                    Ok(permit) => {
                        // Check and send under the same generation/revision lock.
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
        })
        .await;
    }
}
