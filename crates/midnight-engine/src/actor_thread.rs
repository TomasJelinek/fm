//! Session-long backend actors on dedicated named threads. They never occupy
//! the bounded Tokio blocking pool that listings, scans and other transient
//! native work share. If the OS refuses another thread, the pool runs the work
//! instead, as before.
use std::sync::{Arc, Mutex};
use tokio::{sync::oneshot, task::JoinSet};

/// Start `work` now; the task added to `tasks` completes when it returns or
/// unwinds.
pub(crate) fn spawn(name: &str, tasks: &mut JoinSet<()>, work: impl FnOnce() + Send + 'static) {
    match start(name, work) {
        Ok(finished) => {
            tasks.spawn(async move {
                let _ = finished.await;
            });
        }
        Err(work) => {
            tasks.spawn_blocking(work);
        }
    }
}

/// Start `work` when this future is first polled and wait until it returns or
/// unwinds. An unpolled future drops `work` without running it.
pub(crate) async fn run(name: &str, work: impl FnOnce() + Send + 'static) {
    match start(name, work) {
        Ok(finished) => {
            let _ = finished.await;
        }
        Err(work) => {
            let _ = tokio::task::spawn_blocking(work).await;
        }
    }
}

fn start<F: FnOnce() + Send + 'static>(name: &str, work: F) -> Result<oneshot::Receiver<()>, F> {
    let slot = Arc::new(Mutex::new(Some(work)));
    let thread_slot = slot.clone();
    let (done, finished) = oneshot::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            // Dropped on return and on unwind, completing the waiter either way.
            let _done = done;
            let work = thread_slot
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(work) = work {
                work();
            }
        });
    if spawned.is_ok() {
        return Ok(finished);
    }
    // The refused thread never ran, so the work is still in the slot.
    match slot
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        Some(work) => Err(work),
        None => Ok(finished),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actors_run_on_named_threads_and_complete_their_task_even_after_a_panic() {
        let mut tasks = JoinSet::new();
        let (name, observed) = std::sync::mpsc::channel();
        spawn("midnight-test-actor", &mut tasks, move || {
            name.send(std::thread::current().name().map(str::to_owned))
                .unwrap();
        });
        spawn("midnight-test-unwind", &mut tasks, || {
            panic!("actor unwinds")
        });
        let (lazy_name, lazy_observed) = std::sync::mpsc::channel();
        tasks.spawn(run("midnight-test-lazy", move || {
            lazy_name
                .send(std::thread::current().name().map(str::to_owned))
                .unwrap();
        }));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(result) = tasks.join_next().await {
                result.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            observed.recv().unwrap().as_deref(),
            Some("midnight-test-actor")
        );
        assert_eq!(
            lazy_observed.recv().unwrap().as_deref(),
            Some("midnight-test-lazy")
        );
    }
}
