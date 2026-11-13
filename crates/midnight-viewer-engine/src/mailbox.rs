//! Latest-frame output with one coalesced event-loop wake, never one task per raster.
use midnight_core::ViewerOutput;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

type Notify = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Default)]
pub struct Mailbox {
    output: Mutex<Vec<ViewerOutput>>,
    reserve_layout: bool,
    opening: Mutex<Opening>,
    notify: Mutex<Option<Notify>>,
    wake_pending: AtomicBool,
}

#[derive(Default)]
struct Opening {
    finished: bool,
    result: Option<Result<(), String>>,
}

impl Mailbox {
    /// Reserve initialization metadata before an opted-in session can publish other output.
    pub(crate) fn with_layout() -> Self {
        Self {
            reserve_layout: true,
            ..Self::default()
        }
    }

    /// Consume the initial backend content-ready or failure acknowledgement once.
    pub fn take_open_result(&self) -> Option<Result<(), String>> {
        self.opening
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result
            .take()
    }

    /// Resolve once; the caller queues the accompanying frame/status to wake the UI.
    pub(crate) fn finish_open(&self, result: Result<(), String>) {
        let mut opening = self.opening.lock().unwrap_or_else(|e| e.into_inner());
        if !opening.finished {
            opening.finished = true;
            opening.result = Some(result);
        }
    }

    /// Register an event-loop wake. Return true only after queueing the wake;
    /// false leaves the next output able to retry after event-loop shutdown/error.
    pub fn notify_on_ready(&self, notify: Notify) {
        *self.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(notify);
        let ready = !self
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        if ready {
            self.wake();
        }
    }

    pub fn take(&self) -> Vec<ViewerOutput> {
        let mut pending = self.output.lock().unwrap_or_else(|e| e.into_inner());
        self.wake_pending.store(false, Ordering::Release);
        let mut outputs = std::mem::take(&mut *pending);
        if self.reserve_layout
            && let Some(index) = outputs
                .iter()
                .position(|output| matches!(output, ViewerOutput::LayoutSelected { .. }))
        {
            let layout = outputs.remove(index);
            outputs.insert(0, layout);
        }
        outputs
    }

    pub(crate) fn put(&self, output: ViewerOutput) {
        let mut pending = self.output.lock().unwrap_or_else(|e| e.into_inner());
        match &output {
            ViewerOutput::Frame(_) => pending.retain(|old| !matches!(old, ViewerOutput::Frame(_))),
            ViewerOutput::Status(_) => {
                pending.retain(|old| !matches!(old, ViewerOutput::Status(_)))
            }
            ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. } => pending
                .retain(|old| {
                    !matches!(
                        old,
                        ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
                    )
                }),
            ViewerOutput::ImageUnavailable { .. } => {
                pending.retain(|old| !matches!(old, ViewerOutput::ImageUnavailable { .. }))
            }
            ViewerOutput::Closed => pending.clear(),
            ViewerOutput::CopyText(_) => {
                pending.retain(|old| !matches!(old, ViewerOutput::CopyText(_)))
            }
            ViewerOutput::WebShortcut(_) => {}
            ViewerOutput::WebState { .. } => {
                pending.retain(|old| !matches!(old, ViewerOutput::WebState { .. }))
            }
            ViewerOutput::LayoutSelected { .. } => {
                pending.retain(|old| !matches!(old, ViewerOutput::LayoutSelected { .. }))
            }
        }
        // Preserve content, the latest raster control and opted-in layout metadata.
        // A full ordinary-event queue must not strand either side of a handoff.
        let frame = matches!(output, ViewerOutput::Frame(_));
        let control = matches!(
            output,
            ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
        );
        let layout = matches!(output, ViewerOutput::LayoutSelected { .. });
        let reserved = usize::from(
            !frame
                && !pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::Frame(_))),
        ) + usize::from(
            !control
                && !pending.iter().any(|old| {
                    matches!(
                        old,
                        ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
                    )
                }),
        ) + usize::from(
            self.reserve_layout
                && !layout
                && !pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::LayoutSelected { .. })),
        );
        if pending.len() < 8 - reserved {
            match &output {
                ViewerOutput::Frame(_) => self.finish_open(Ok(())),
                ViewerOutput::ImageUnavailable { message, .. } => {
                    self.finish_open(Err(message.clone()))
                }
                ViewerOutput::Closed => {
                    self.finish_open(Err("Viewer closed before displaying content.".into()))
                }
                _ => {}
            }
            pending.push(output);
        }
        drop(pending);
        self.wake();
    }

    fn wake(&self) {
        let notify = self
            .notify
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(notify) = notify
            && !self.wake_pending.swap(true, Ordering::AcqRel)
            && !notify()
        {
            self.wake_pending.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn late_subscriber_and_output_bursts_wake_once_until_the_ui_drains() {
        let mailbox = Mailbox::default();
        mailbox.put(ViewerOutput::Status("already ready".into()));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        mailbox.notify_on_ready(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            true
        }));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "late subscriber must not wait for polling"
        );
        for number in 0..1000 {
            mailbox.put(ViewerOutput::Status(number.to_string()));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1, "bounded event-loop queue");
        assert!(
            matches!(mailbox.take().as_slice(), [ViewerOutput::Status(value)] if value == "999")
        );
        mailbox.put(ViewerOutput::Closed);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn rejected_wake_keeps_latest_output_and_allows_the_next_output_to_retry() {
        let mailbox = Mailbox::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        mailbox.notify_on_ready(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            false
        }));
        mailbox.put(ViewerOutput::Status("one".into()));
        mailbox.put(ViewerOutput::Status("two".into()));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(
            matches!(mailbox.take().as_slice(), [ViewerOutput::Status(value)] if value == "two")
        );
    }
}

#[cfg(test)]
#[test]
fn raster_failures_remain_bounded_and_preserve_the_latest_surface_authority() {
    let mailbox = Mailbox::default();
    for surface in 1..=10_000 {
        mailbox.put(ViewerOutput::RasterUnavailable {
            surface: midnight_core::ViewerSurfaceId(surface),
            message: "guard failed".into(),
        });
    }
    let output = mailbox.take();
    assert_eq!(output.len(), 1);
    assert!(matches!(
        output[0],
        ViewerOutput::RasterUnavailable {
            surface: midnight_core::ViewerSurfaceId(10_000),
            ..
        }
    ));
}

#[cfg(test)]
#[test]
fn raster_rebase_delivery_survives_full_event_pressure_and_coalesces_to_latest() {
    let mailbox = Mailbox::default();
    for _ in 0..100 {
        mailbox.put(ViewerOutput::WebShortcut(
            midnight_core::viewer_web::HostShortcut::Find,
        ));
    }
    for transition in 1..=10_000 {
        mailbox.put(ViewerOutput::RasterRebase {
            surface: midnight_core::ViewerSurfaceId(1),
            transition,
        });
    }
    let output = mailbox.take();
    assert!(output.len() <= 8);
    assert_eq!(
        output
            .iter()
            .filter(|event| matches!(event, ViewerOutput::RasterRebase { .. }))
            .count(),
        1
    );
    assert!(output.iter().any(|event| matches!(
        event,
        ViewerOutput::RasterRebase {
            transition: 10_000,
            ..
        }
    )));
    mailbox.put(ViewerOutput::RasterRebase {
        surface: midnight_core::ViewerSurfaceId(1),
        transition: 10_001,
    });
    mailbox.put(ViewerOutput::Closed);
    assert!(matches!(mailbox.take().as_slice(), [ViewerOutput::Closed]));
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use midnight_core::{VIEWER_MODE_HTML, ViewerFrame, ViewerSurfaceId, viewer_web::HostShortcut};
    use std::sync::atomic::AtomicUsize;

    fn frame() -> ViewerOutput {
        ViewerOutput::Frame(Box::new(ViewerFrame {
            generation: 7,
            position_sequence: [0; 2],
            revision: 1,
            mode: VIEWER_MODE_HTML,
            modes: vec![VIEWER_MODE_HTML],
            wrap_enabled: false,
            wrap_available: false,
            image: None,
            raster: None,
            raster_rebase: None,
            width: 1,
            height: 1,
            pixels: Arc::from([0, 0, 0, 255]),
            tiles: None,
            accessible_text: "HTML fixture".into(),
            position: 0.0,
            vertical_extent: 1.0,
            horizontal_position: 0.0,
            horizontal_extent: 1.0,
            caption: "HTML fixture".into(),
            status: "Offline".into(),
        }))
    }

    #[test]
    fn layout_selection_survives_full_event_pressure_with_frame_and_raster_control() {
        let mailbox = Mailbox::with_layout();
        for _ in 0..100 {
            mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
        }
        mailbox.put(frame());
        mailbox.put(ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(3),
            transition: 41,
        });
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 19,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });

        let outputs = mailbox.take();
        assert_eq!(outputs.len(), 8);
        assert!(matches!(
            outputs.first(),
            Some(ViewerOutput::LayoutSelected {
                token: 19,
                mode: VIEWER_MODE_HTML,
                content_generation: 7,
            })
        ));
        assert_eq!(
            outputs
                .iter()
                .filter(|output| matches!(output, ViewerOutput::WebShortcut(HostShortcut::Find)))
                .count(),
            5
        );
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, ViewerOutput::Frame(_)))
        );
        assert!(outputs.iter().any(|output| matches!(
            output,
            ViewerOutput::RasterRebase {
                surface: ViewerSurfaceId(3),
                transition: 41,
            }
        )));
    }

    #[test]
    fn layout_selection_replaces_pending_token_under_pressure() {
        let mailbox = Mailbox::with_layout();
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 1,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });
        for _ in 0..100 {
            mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
        }
        mailbox.put(frame());
        mailbox.put(ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(3),
            transition: 41,
        });
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 2,
            mode: 2,
            content_generation: 8,
        });

        let outputs = mailbox.take();
        assert_eq!(outputs.len(), 8);
        assert!(matches!(
            outputs.first(),
            Some(ViewerOutput::LayoutSelected {
                token: 2,
                mode: 2,
                content_generation: 8,
            })
        ));
        assert_eq!(
            outputs
                .iter()
                .filter(|output| matches!(output, ViewerOutput::LayoutSelected { .. }))
                .count(),
            1
        );
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, ViewerOutput::Frame(_)))
        );
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, ViewerOutput::RasterRebase { transition: 41, .. }))
        );
    }

    #[test]
    fn layout_selection_precedes_same_batch_frame_without_reordering_other_outputs() {
        let mailbox = Mailbox::with_layout();
        mailbox.put(frame());
        mailbox.put(ViewerOutput::Status("content ready".into()));
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 19,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });
        assert!(matches!(
            mailbox.take().as_slice(),
            [
                ViewerOutput::LayoutSelected { token: 19, .. },
                ViewerOutput::Frame(_),
                ViewerOutput::Status(message),
            ] if message == "content ready"
        ));
    }

    #[test]
    fn layout_selection_leaves_startup_waiting_until_a_real_frame() {
        let mailbox = Mailbox::with_layout();
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 19,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });
        assert!(mailbox.take_open_result().is_none());
        assert!(matches!(
            mailbox.take().as_slice(),
            [ViewerOutput::LayoutSelected { token: 19, .. }]
        ));
        assert!(mailbox.take_open_result().is_none());
        mailbox.put(frame());
        assert_eq!(mailbox.take_open_result(), Some(Ok(())));
        assert!(mailbox.take_open_result().is_none());
    }

    #[test]
    fn closed_clears_pending_layout_and_reports_unfinished_opening() {
        let mailbox = Mailbox::with_layout();
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 19,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });
        for _ in 0..100 {
            mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
        }
        mailbox.put(ViewerOutput::Closed);
        assert!(matches!(mailbox.take().as_slice(), [ViewerOutput::Closed]));
        assert!(matches!(mailbox.take_open_result(), Some(Err(_))));
    }

    #[test]
    fn fixed_viewport_mailbox_keeps_six_ordinary_events() {
        let mailbox = Mailbox::default();
        for _ in 0..100 {
            mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
        }
        mailbox.put(frame());
        mailbox.put(ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(3),
            transition: 41,
        });
        let outputs = mailbox.take();
        assert_eq!(outputs.len(), 8);
        assert_eq!(
            outputs
                .iter()
                .filter(|output| matches!(output, ViewerOutput::WebShortcut(HostShortcut::Find)))
                .count(),
            6
        );
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, ViewerOutput::Frame(_)))
        );
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, ViewerOutput::RasterRebase { transition: 41, .. }))
        );
    }

    #[test]
    fn layout_selection_bursts_keep_wakes_coalesced() {
        let mailbox = Mailbox::with_layout();
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 1,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        mailbox.notify_on_ready(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            true
        }));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        for token in 2..=100 {
            mailbox.put(ViewerOutput::LayoutSelected {
                token,
                mode: VIEWER_MODE_HTML,
                content_generation: 7,
            });
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            mailbox.take().as_slice(),
            [ViewerOutput::LayoutSelected { token: 100, .. }]
        ));
        mailbox.put(ViewerOutput::LayoutSelected {
            token: 101,
            mode: VIEWER_MODE_HTML,
            content_generation: 7,
        });
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }
}
