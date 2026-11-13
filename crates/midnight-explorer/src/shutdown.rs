//! App-owned runtime cleanup after a window attempt returns or unwinds.

use crate::startup_recovery::Attempt;

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
