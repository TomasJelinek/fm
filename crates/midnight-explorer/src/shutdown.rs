//! App-owned runtime cleanup after a window attempt returns or unwinds.

use crate::startup_recovery::Attempt;
use midnight_engine::reclaim::{self, Retired};
use tokio::{sync::mpsc, time::Instant};

/// Hand the last retired allocations to the reclaimer and await it, never past `deadline`:
/// a destructor blocked in native cleanup (e.g. a handle on an unreachable share) keeps
/// its blocking worker, not the hidden process. A batch that cannot be queued in time is
/// dropped on a detached blocking worker, never on this thread; the bounded runtime
/// shutdown then limits both.
pub(crate) async fn retire_final(
    queue: &mpsc::Sender<Retired>,
    retired: Retired,
    deadline: Instant,
) {
    match tokio::time::timeout_at(deadline, queue.reserve()).await {
        Ok(Ok(permit)) => {
            permit.send(retired);
            let _ = tokio::time::timeout_at(deadline, reclaim::barrier(queue)).await;
        }
        Ok(Err(_)) | Err(_) => drop(tokio::task::spawn_blocking(move || drop(retired))),
    }
}

pub(crate) fn finish_after_unwind<T>(
    outcome: std::thread::Result<Result<Attempt, String>>,
    retire_spare: impl FnOnce() -> (T, Result<(), String>),
    shutdown_runtime: impl FnOnce(T),
    retry: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match outcome {
        Ok(result) => {
            crate::viewers::finish_after_image_spare(result, retire_spare, shutdown_runtime, retry)
        }
        Err(payload) => {
            // A retained UI callback may still own a Service clone after Slint
            // unwinds. Explicitly stop its blocking image owner before consuming
            // the runtime through the caller's bounded shutdown path.
            let (runtime, retirement) = retire_spare();
            shutdown_runtime(runtime);
            if let Err(error) = retirement {
                eprintln!("Midnight Explorer: {error}");
            }
            // Cleanup must neither replace the original panic nor qualify it for
            // the normal renderer-startup software retry.
            std::panic::resume_unwind(payload)
        }
    }
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
