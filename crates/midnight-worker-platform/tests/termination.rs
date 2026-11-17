//! Confirms termination independently of the expected nonzero exit from forced shutdown.
#[test]
fn killed_worker_has_a_positive_termination_receipt_without_a_success_exit_status() {
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_midnight-worker-probe"));
    let mut worker = midnight_worker_platform::spawn(
        executable,
        midnight_worker_platform::Limits {
            memory_bytes: 512 * 1024 * 1024,
            cpu_seconds: 15,
        },
    )
    .unwrap();
    // Startup may already have failed on an unsupported sandbox. In either case the
    // actual native wait must positively acknowledge termination, never infer it from kill.
    let _ = worker.process.kill();
    worker.process.wait_terminated().unwrap();
    drop(worker);
}
