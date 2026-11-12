//! Bounded catalog startup; all transports must be known before duplicate-ID routing.

use std::sync::atomic::{AtomicBool, Ordering};

/// Admission is shared by service clones; startup can schedule only one speculative task.
#[derive(Default)]
pub(crate) struct Preparation(AtomicBool);

impl Preparation {
    pub fn start(&self, schedule: impl FnOnce()) -> bool {
        if self.0.swap(true, Ordering::AcqRel) {
            return false;
        }
        schedule();
        true
    }
}
pub(crate) fn parallel<A, B: Send, C: Send>(
    native: impl FnOnce() -> Result<A, String>,
    images: impl FnOnce() -> Result<B, String> + Send,
    web: impl FnOnce() -> Result<C, String> + Send,
) -> Result<(A, B, C), String> {
    std::thread::scope(|scope| {
        let images = std::thread::Builder::new()
            .name("viewer-image-catalog".into())
            .spawn_scoped(scope, images)
            .map_err(|error| error.to_string())?;
        let web = std::thread::Builder::new()
            .name("viewer-web-catalog".into())
            .spawn_scoped(scope, web)
            .map_err(|error| error.to_string())?;
        let native = native();
        // Join both workers even after an error. No discovery or its source ownership
        // survives the bounded startup phase, including cancellation/failed opening.
        let images = images
            .join()
            .map_err(|_| "Image catalog discovery panicked".to_owned());
        let web = web
            .join()
            .map_err(|_| "Web catalog discovery panicked".to_owned());
        Ok((native?, images??, web??))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn independent_catalogs_begin_before_the_native_catalog_finishes() {
        let (started, ready) = mpsc::sync_channel(2);
        let images = started.clone();
        let result = parallel(
            || {
                for _ in 0..2 {
                    ready
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|_| "catalog startup was serialized".to_owned())?;
                }
                Ok("native")
            },
            move || {
                images.send(()).unwrap();
                Ok("images")
            },
            move || {
                started.send(()).unwrap();
                Ok("web")
            },
        )
        .unwrap();
        assert_eq!(result, ("native", "images", "web"));
    }

    #[test]
    fn failure_waits_for_other_catalog_work_before_returning() {
        let completed = Arc::new(AtomicUsize::new(0));
        let images = completed.clone();
        let web = completed.clone();
        let result = parallel(
            || Err::<(), _>("native catalog failed".to_owned()),
            move || {
                images.fetch_add(1, Ordering::Release);
                Ok(())
            },
            move || {
                web.fetch_add(1, Ordering::Release);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err(), "native catalog failed");
        assert_eq!(completed.load(Ordering::Acquire), 2);
        assert_eq!(
            Arc::strong_count(&completed),
            1,
            "startup task ownership must be released"
        );
    }
    #[test]
    fn repeated_and_concurrent_preparation_requests_schedule_exactly_one_task() {
        let preparation = Preparation::default();
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    preparation.start(|| {
                        calls.fetch_add(1, Ordering::Release);
                    });
                });
            }
        });
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert!(!preparation.start(|| panic!("preparation must remain one-shot after completion")));
    }
}
