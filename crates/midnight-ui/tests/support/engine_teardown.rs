//! Deadlines and the Engine shutdown drain shared by the real-Engine UI harnesses
//! (support/tree_filters.rs and support/operations_native.rs), so both keep one
//! documented timing policy and one teardown protocol.
//!
//! Teardown never panics: harnesses usually drop while a failed assertion unwinds,
//! and a second panic in `Drop` would abort the whole test binary. Errors are
//! returned, and the caller asserts them only after `Runtime::shutdown_timeout`
//! (dropping a runtime while panicking waits for its blocking pool without limit)
//! and only when the thread is not already panicking.
#![allow(dead_code)] // Each harness uses its own subset.
use midnight_core::{Command, Event};
use midnight_engine::{CommandSender, reclaim};
use std::time::Duration;
use tokio::{sync::mpsc, task::JoinHandle};

/// Deadline of one harness wait for UI or Engine progress. It only elapses when a
/// test fails, so it is generous for loaded CI runners.
pub const UI_WAIT: Duration = Duration::from_secs(15);
/// Deadline for an orderly Engine shutdown after the presenter retired its state.
pub const ENGINE_SHUTDOWN: Duration = Duration::from_secs(8);
/// Bound for blocking-pool work still running when a harness drops its runtime.
pub const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(3);

/// Send `Shutdown` and keep moving Engine events into the bounded reclamation queue
/// until the Engine task ends (`observe` sees each event first).
pub async fn shutdown(
    commands: &CommandSender,
    engine: &mut JoinHandle<()>,
    events: &mut mpsc::Receiver<Event>,
    retire_queue: &mpsc::Sender<reclaim::Retired>,
    mut observe: impl FnMut(&Event),
) -> Result<(), String> {
    let drain = async {
        let shutdown = commands.send(Command::Shutdown);
        tokio::pin!(shutdown);
        let mut sent = false;
        loop {
            // Final closed-window ownership goes to the same bounded worker queue
            // before another event may be taken.
            let Ok(permit) = retire_queue.reserve().await else {
                return Err("the reclamation queue closed during shutdown".to_owned());
            };
            tokio::select! {
                // An Engine that already ended is reported before anything else.
                biased;
                result = &mut *engine => {
                    return result.map_err(|error| format!("Engine task failed: {error}"));
                }
                // A refused send means the Engine dropped its receiver; the branch
                // above then reports how its task ended.
                _ = &mut shutdown, if !sent => sent = true,
                event = events.recv() => {
                    let Some(event) = event else {
                        return (&mut *engine)
                            .await
                            .map_err(|error| format!("Engine task failed: {error}"));
                    };
                    observe(&event);
                    permit.send(vec![Box::new(event)]);
                }
            }
        }
    };
    tokio::time::timeout(ENGINE_SHUTDOWN, drain)
        .await
        .unwrap_or_else(|_| Err(format!("Engine shutdown exceeded {ENGINE_SHUTDOWN:?}")))
}

/// Hand values to the reclamation worker; if it is gone, drop them on a blocking
/// thread instead of the UI thread. Each wait is bounded by [`ENGINE_SHUTDOWN`]: a
/// stuck worker or drop is reported, and the caller's `Runtime::shutdown_timeout`
/// bounds the blocking drop it leaves behind.
pub async fn retire_final(
    queue: &mpsc::Sender<reclaim::Retired>,
    retired: reclaim::Retired,
) -> Result<(), String> {
    let retired = match tokio::time::timeout(ENGINE_SHUTDOWN, queue.reserve()).await {
        Ok(Ok(permit)) => {
            permit.send(retired);
            return Ok(());
        }
        Ok(Err(_closed)) => retired,
        Err(_elapsed) => {
            drop(tokio::task::spawn_blocking(move || drop(retired)));
            return Err(format!(
                "the reclamation queue stayed full for {ENGINE_SHUTDOWN:?}"
            ));
        }
    };
    let dropped = tokio::task::spawn_blocking(move || drop(retired));
    match tokio::time::timeout(ENGINE_SHUTDOWN, dropped).await {
        Ok(result) => result.map_err(|error| format!("dropping retired values failed: {error}")),
        Err(_elapsed) => Err(format!(
            "dropping retired values exceeded {ENGINE_SHUTDOWN:?}"
        )),
    }
}

/// After [`shutdown`]: retire the unread events and wait (bounded) until the
/// reclamation worker dropped everything handed to it.
pub async fn retire_remaining(
    queue: &mpsc::Sender<reclaim::Retired>,
    events: mpsc::Receiver<Event>,
) -> Result<(), String> {
    let mut retired = reclaim::Retired::new();
    reclaim::retire_receiver(events, &mut retired);
    retire_final(queue, retired).await?;
    tokio::time::timeout(ENGINE_SHUTDOWN, reclaim::barrier(queue))
        .await
        .map_err(|_elapsed| format!("the reclamation barrier exceeded {ENGINE_SHUTDOWN:?}"))?
        .map_err(|error| format!("reclamation barrier failed: {error:?}"))
}
