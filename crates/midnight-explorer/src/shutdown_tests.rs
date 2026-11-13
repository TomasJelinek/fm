use super::*;
use std::{
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    time::Duration,
};

/// A failing regression must still stop its actual blocking owner and consume
/// Tokio through bounded shutdown, rather than hanging the whole test process.
struct EmergencyCleanup<'a> {
    runtime: &'a RefCell<Option<tokio::runtime::Runtime>>,
    retirement: &'a midnight_viewer_engine::ImageSpareRetirement,
}

impl Drop for EmergencyCleanup<'_> {
    fn drop(&mut self) {
        self.retirement.stop_spare();
        if let Some(runtime) = self.runtime.borrow_mut().take() {
            let _ = runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), self.retirement.wait()).await
            });
            runtime.shutdown_timeout(Duration::from_millis(100));
        }
    }
}

#[test]
fn caught_panic_retires_actual_idle_image_owner_before_shutdown_with_a_live_service_clone() {
    let runtime = RefCell::new(Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap(),
    ));
    let service = midnight_viewer_engine::Service::with_native_capture(
        runtime.borrow().as_ref().unwrap().handle().clone(),
        Arc::new(|_, _| Err("The idle shutdown fixture must not capture any source.".into())),
    );
    let retirement = service.image_spare_retirement();
    let retained_service = service.clone();
    drop(service);
    let emergency = EmergencyCleanup {
        runtime: &runtime,
        retirement: &retirement,
    };
    let token = Arc::new(());
    let payload = token.clone();
    let outcome = catch_unwind(AssertUnwindSafe(move || -> Result<Attempt, String> {
        std::panic::panic_any(payload)
    }));
    let phases = RefCell::new(Vec::new());
    let retried = Cell::new(false);
    let resumed = catch_unwind(AssertUnwindSafe(|| {
        finish_after_unwind(
            outcome,
            || {
                phases.borrow_mut().push("spare stop");
                retirement.stop_spare();
                let result = runtime.borrow().as_ref().unwrap().block_on(async {
                    tokio::time::timeout(Duration::from_secs(2), retirement.wait())
                        .await
                        .map_err(|_| {
                            "idle image owner did not stop before the test deadline".to_owned()
                        })?
                });
                if result.is_ok() {
                    phases.borrow_mut().push("spare owner exited");
                }
                (runtime.borrow_mut().take().unwrap(), result)
            },
            |runtime| {
                phases.borrow_mut().push("runtime shutdown");
                runtime.shutdown_timeout(Duration::from_millis(100));
            },
            || {
                retried.set(true);
                Ok(())
            },
        )
    }));
    // Emergency cleanup happens before assertions, including on the intentionally
    // broken implementation. Its work never contributes to production's phases.
    drop(emergency);
    let resumed = resumed.unwrap_err().downcast::<Arc<()>>().unwrap();
    assert!(Arc::ptr_eq(&token, &resumed));
    assert!(
        !retried.get(),
        "a caught panic must never trigger software retry"
    );
    assert_eq!(
        *phases.borrow(),
        ["spare stop", "spare owner exited", "runtime shutdown"],
    );
    // Keep the external Control alive through retirement and runtime cleanup.
    drop(retained_service);
}

#[test]
fn failed_panic_retirement_still_shuts_down_and_resumes_the_original_payload_without_retry() {
    let token = Arc::new(());
    let phases = RefCell::new(Vec::new());
    let retried = Cell::new(false);
    let resumed = catch_unwind(AssertUnwindSafe(|| {
        finish_after_unwind(
            Err(Box::new(token.clone())),
            || {
                phases.borrow_mut().push("spare failed");
                (41u8, Err("OwnerFailed; cleanup unconfirmed".into()))
            },
            |runtime| {
                assert_eq!(runtime, 41);
                phases.borrow_mut().push("runtime shutdown");
            },
            || {
                retried.set(true);
                Ok(())
            },
        )
    }));
    let resumed = resumed.unwrap_err().downcast::<Arc<()>>().unwrap();
    assert!(Arc::ptr_eq(&token, &resumed));
    assert!(!retried.get());
    assert_eq!(*phases.borrow(), ["spare failed", "runtime shutdown"]);
}

#[test]
fn normal_results_preserve_existing_cleanup_error_and_software_retry_behavior() {
    for (attempt, expected_error, retry_expected) in [
        (Ok(Attempt::Finished), None, false),
        (
            Err("startup failed".to_owned()),
            Some("startup failed"),
            false,
        ),
        (
            Ok(Attempt::NativeStartupFailed("renderer unavailable".into())),
            None,
            true,
        ),
    ] {
        let phases = RefCell::new(Vec::new());
        let result = finish_after_unwind(
            Ok(attempt),
            || {
                phases.borrow_mut().push("spare retired");
                (41u8, Ok(()))
            },
            |runtime| {
                assert_eq!(runtime, 41);
                phases.borrow_mut().push("runtime shutdown");
            },
            || {
                phases.borrow_mut().push("software retry");
                Ok(())
            },
        );
        assert_eq!(result.err().as_deref(), expected_error);
        let mut expected = vec!["spare retired", "runtime shutdown"];
        if retry_expected {
            expected.push("software retry");
        }
        assert_eq!(*phases.borrow(), expected);
    }
}
