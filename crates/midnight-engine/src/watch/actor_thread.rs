//! Session-long backend actors on dedicated named threads. They never occupy
//! the bounded Tokio blocking pool that listings, scans and other transient
//! native work share.
use std::sync::{Arc, Mutex};
use tokio::{sync::oneshot, task::JoinSet};

/// Run `work` on its own thread. The task added to `tasks` completes when the
/// work returns or unwinds. If the OS refuses another thread, the blocking pool
/// runs the work instead, as before.
pub(crate) fn spawn(name: &str, tasks: &mut JoinSet<()>, work: impl FnOnce() + Send + 'static) {
    let slot = Arc::new(Mutex::new(Some(work)));
    let thread_slot = slot.clone();
    let (done, finished) = oneshot::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            // Dropped on return and on unwind, completing the task either way.
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
        tasks.spawn(async move {
            let _ = finished.await;
        });
    } else if let Some(work) = slot
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        tasks.spawn_blocking(work);
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
    }
}
