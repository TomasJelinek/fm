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
    closed: AtomicBool,
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
        if let Some(index) = outputs
            .iter()
            .position(|output| matches!(output, ViewerOutput::PaletteApplied { .. }))
        {
            let palette = outputs.remove(index);
            let after_layout = usize::from(matches!(
                outputs.first(),
                Some(ViewerOutput::LayoutSelected { .. })
            ));
            outputs.insert(after_layout, palette);
        }
        if let Some(failure) = outputs
            .iter()
            .position(|output| matches!(output, ViewerOutput::VerticalPositionUnavailable { .. }))
            && let Some(frame) = outputs[..failure]
                .iter()
                .position(|output| matches!(output, ViewerOutput::Frame(_)))
        {
            let failure = outputs.remove(failure);
            outputs.insert(frame, failure);
        }
        outputs
    }

    /// Fence pending frames and controls when a video seek or file switch is accepted.
    pub(crate) fn discard_pending_video(&self) {
        let retired: Vec<_> = {
            let mut pending = self
                .output
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            pending
                .extract_if(.., |output| {
                    matches!(output, ViewerOutput::MediaSnapshot(_))
                        || matches!(output, ViewerOutput::Frame(frame)
                        if frame.mode == midnight_core::VIEWER_MODE_VIDEO)
                })
                .collect()
        };
        drop(retired);
    }

    pub(crate) fn put(&self, output: ViewerOutput) {
        let mut pending = self
            .output
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.closed.load(Ordering::Acquire) {
            drop(pending);
            drop(output);
            return;
        }
        if let ViewerOutput::PaletteApplied { revision, generation, .. } = &output
            && pending.iter().any(|old| matches!(old,
                ViewerOutput::PaletteApplied { revision: old_revision, generation: old_generation, .. }
                    if (*old_revision, *old_generation) > (*revision, *generation)))
        {
            drop(pending);
            drop(output);
            return;
        }
        if let ViewerOutput::VerticalPositionUnavailable {
            generation,
            sequence,
        } = &output
            && pending.iter().any(|old| matches!(old,
                ViewerOutput::VerticalPositionUnavailable { generation: old_generation, sequence: old_sequence }
                    if (*old_generation, *old_sequence) >= (*generation, *sequence)))
        {
            drop(pending);
            self.wake();
            return;
        }
        if let ViewerOutput::MediaSnapshot(snapshot) = &output
            && pending.iter().any(|old| {
                matches!(old, ViewerOutput::MediaSnapshot(previous)
                    if (previous.session, previous.revision) >= (snapshot.session, snapshot.revision))
            })
        {
            drop(pending);
            self.wake();
            return;
        }
        if let ViewerOutput::MediaRetired { session } = &output
            && pending.iter().any(|old| {
                matches!(old, ViewerOutput::MediaRetired { session: previous }
                    if previous >= session)
            })
        {
            drop(pending);
            self.wake();
            return;
        }
        let mut retired = Vec::new();
        let mut index = 0;
        while index < pending.len() {
            let replace = match &output {
                ViewerOutput::Frame(_) => matches!(pending[index], ViewerOutput::Frame(_)),
                ViewerOutput::PaletteApplied { .. } => {
                    matches!(pending[index], ViewerOutput::PaletteApplied { .. })
                }
                ViewerOutput::Status(_) => matches!(pending[index], ViewerOutput::Status(_)),
                ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. } => {
                    matches!(
                        pending[index],
                        ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
                    )
                }
                ViewerOutput::ImageUnavailable { .. } => {
                    matches!(pending[index], ViewerOutput::ImageUnavailable { .. })
                }
                ViewerOutput::SandboxNotice(_) => {
                    matches!(pending[index], ViewerOutput::SandboxNotice(_))
                }
                ViewerOutput::LinkHover(_) => matches!(pending[index], ViewerOutput::LinkHover(_)),
                ViewerOutput::LinkFocus { .. } => {
                    matches!(pending[index], ViewerOutput::LinkFocus { .. })
                }
                ViewerOutput::LinkHistory { .. } => {
                    matches!(pending[index], ViewerOutput::LinkHistory { .. })
                }
                ViewerOutput::WebCursor(_) => matches!(pending[index], ViewerOutput::WebCursor(_)),
                ViewerOutput::LinkRequest(_) => false,
                ViewerOutput::CopyText(_) => matches!(pending[index], ViewerOutput::CopyText(_)),
                ViewerOutput::Copying(_) => matches!(pending[index], ViewerOutput::Copying(_)),
                ViewerOutput::WebState { .. } => {
                    matches!(pending[index], ViewerOutput::WebState { .. })
                }
                ViewerOutput::MediaSnapshot(_) => {
                    matches!(pending[index], ViewerOutput::MediaSnapshot(_))
                }
                ViewerOutput::MediaRetired { .. } => {
                    matches!(pending[index], ViewerOutput::MediaRetired { .. })
                }
                ViewerOutput::LayoutSelected { .. } => {
                    matches!(pending[index], ViewerOutput::LayoutSelected { .. })
                }
                ViewerOutput::VerticalPositionUnavailable { .. } => {
                    matches!(
                        pending[index],
                        ViewerOutput::VerticalPositionUnavailable { .. }
                    )
                }
                ViewerOutput::Closed => true,
                ViewerOutput::WebShortcut(_) => false,
            };
            if replace {
                retired.push(pending.remove(index));
            } else {
                index += 1;
            }
        }
        let frame = matches!(output, ViewerOutput::Frame(_));
        let raster = matches!(
            output,
            ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
        );
        let layout = matches!(output, ViewerOutput::LayoutSelected { .. });
        let palette = matches!(output, ViewerOutput::PaletteApplied { .. });
        let failure = matches!(output, ViewerOutput::VerticalPositionUnavailable { .. });
        let media_retired = matches!(output, ViewerOutput::MediaRetired { .. });
        // The latest link hover and focus, and every followed link's request, are
        // admitted like the notice: the window waits for the hover's answer, and a
        // request is what the user asked for. So are the latest copied text, which may
        // have taken a whole-file read, and whether such a read runs, which decides what
        // Escape does in the window.
        let notice = matches!(
            output,
            ViewerOutput::SandboxNotice(_)
                | ViewerOutput::LinkHover(_)
                | ViewerOutput::LinkFocus { .. }
                | ViewerOutput::LinkHistory { .. }
                | ViewerOutput::WebCursor(_)
                | ViewerOutput::LinkRequest(_)
                | ViewerOutput::CopyText(_)
                | ViewerOutput::Copying(_)
        );
        let reserved = usize::from(
            !frame
                && !pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::Frame(_))),
        ) + usize::from(
            !raster
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
        ) + usize::from(
            !palette
                && !pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::PaletteApplied { .. })),
        );
        // Coalesced position failures, media retirement and the sandbox notice each have
        // their own slot: neither ordinary output nor one another can lose these fences,
        // or the notice whose Install… action the window offers.
        let occupied = pending.len()
            - usize::from(
                pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::VerticalPositionUnavailable { .. })),
            )
            - usize::from(
                pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::MediaRetired { .. })),
            )
            - usize::from(
                pending
                    .iter()
                    .any(|old| matches!(old, ViewerOutput::SandboxNotice(_))),
            );
        if failure || media_retired || notice || occupied < 8 - reserved {
            match &output {
                ViewerOutput::Frame(_) => self.finish_open(Ok(())),
                ViewerOutput::ImageUnavailable { message, .. } => {
                    self.finish_open(Err(message.clone()))
                }
                ViewerOutput::Closed => {
                    self.closed.store(true, Ordering::Release);
                    self.finish_open(Err("Viewer closed before displaying content.".into()));
                }
                _ => {}
            }
            pending.push(output);
        } else {
            retired.push(output);
        }
        drop(pending);
        drop(retired);
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
#[test]
fn a_sandbox_notice_survives_full_event_pressure_and_the_latest_replaces_it() {
    let notice = |message: &str| {
        ViewerOutput::SandboxNotice(Some(midnight_core::ViewerSandboxNotice {
            message: message.into(),
            installable: true,
            installing: false,
            details: None,
        }))
    };
    let mailbox = Mailbox::default();
    mailbox.put(notice("refused"));
    for _ in 0..100 {
        mailbox.put(ViewerOutput::WebShortcut(
            midnight_core::viewer_web::HostShortcut::Find,
        ));
    }
    mailbox.put(notice("waiting"));
    let notices: Vec<_> = mailbox
        .take()
        .into_iter()
        .filter(|output| matches!(output, ViewerOutput::SandboxNotice(_)))
        .collect();
    assert!(
        matches!(notices.as_slice(), [ViewerOutput::SandboxNotice(Some(notice))]
            if notice.message == "waiting"),
        "{notices:?}"
    );
    // Removing it is never lost either.
    for _ in 0..100 {
        mailbox.put(ViewerOutput::WebShortcut(
            midnight_core::viewer_web::HostShortcut::Find,
        ));
    }
    mailbox.put(ViewerOutput::SandboxNotice(None));
    assert!(
        mailbox
            .take()
            .iter()
            .any(|output| matches!(output, ViewerOutput::SandboxNotice(None)))
    );
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use midnight_core::{VIEWER_MODE_HTML, ViewerFrame, ViewerSurfaceId, viewer_web::HostShortcut};
    use std::sync::atomic::AtomicUsize;

    fn frame() -> ViewerOutput {
        ViewerOutput::Frame(Box::new(ViewerFrame {
            generation: 7,
            media_control: None,
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
    fn media_retirement_survives_saturation_discard_and_coalesces_to_latest_session() {
        for retire_first in [false, true] {
            let mailbox = Mailbox::with_layout();
            let retire = |session| ViewerOutput::MediaRetired {
                session: midnight_core::MediaSessionId(session),
            };
            if retire_first {
                mailbox.put(retire(3));
            }
            for _ in 0..100 {
                mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
            }
            mailbox.put(ViewerOutput::LayoutSelected {
                token: 19,
                mode: VIEWER_MODE_HTML,
                content_generation: 9,
            });
            mailbox.put(ViewerOutput::PaletteApplied {
                revision: 5,
                generation: 9,
                minimum_frame_revision: Some(1),
            });
            mailbox.put(ViewerOutput::RasterRebase {
                surface: ViewerSurfaceId(3),
                transition: 41,
            });
            mailbox.put(frame());
            mailbox.put(ViewerOutput::VerticalPositionUnavailable {
                generation: 9,
                sequence: 12,
            });
            for session in [3, 7, 5, 7] {
                mailbox.put(retire(session));
            }
            mailbox.discard_pending_video();
            let outputs = mailbox.take();
            assert_eq!(
                outputs.len(),
                10,
                "ordinary eight plus two reserved controls"
            );
            assert!(matches!(outputs[0], ViewerOutput::LayoutSelected { .. }));
            assert_eq!(
                outputs
                    .iter()
                    .filter(|output| matches!(output, ViewerOutput::MediaRetired { .. }))
                    .count(),
                1
            );
            assert!(outputs.iter().any(|output| matches!(
                output, ViewerOutput::MediaRetired { session } if session.0 == 7
            )));
            assert!(outputs.iter().any(|output| matches!(
                output,
                ViewerOutput::VerticalPositionUnavailable { sequence: 12, .. }
            )));
            assert_eq!(
                outputs
                    .iter()
                    .filter(|output| matches!(output, ViewerOutput::WebShortcut(_)))
                    .count(),
                4
            );
            mailbox.put(retire(9));
            mailbox.put(ViewerOutput::Closed);
            mailbox.put(retire(10));
            assert!(matches!(mailbox.take().as_slice(), [ViewerOutput::Closed]));
        }
    }

    #[test]
    fn video_state_coalesces_and_seek_or_navigation_retires_its_pending_pixels() {
        let mailbox = Mailbox::default();
        let state = |generation, revision, playing| {
            let mut snapshot =
                crate::video_actor::opening(midnight_core::MediaSessionId(generation));
            snapshot.revision = revision;
            snapshot.playing = playing;
            ViewerOutput::MediaSnapshot(snapshot)
        };
        mailbox.put(state(2, 1, true));
        mailbox.put(state(1, 3, true));
        mailbox.put(state(2, 3, false));
        mailbox.put(state(2, 2, true));
        assert!(matches!(
            mailbox.take().as_slice(),
            [ViewerOutput::MediaSnapshot(snapshot)]
                if snapshot.session.0 == 2 && snapshot.revision == 3 && !snapshot.playing
        ));
        let ViewerOutput::Frame(mut pixels) = frame() else {
            unreachable!()
        };
        pixels.mode = midnight_core::VIEWER_MODE_VIDEO;
        pixels.media_control = Some(midnight_core::MediaControlId(0));
        mailbox.put(ViewerOutput::Frame(pixels));
        mailbox.put(state(2, 4, false));
        mailbox.put(ViewerOutput::Status("seeking".into()));
        mailbox.discard_pending_video();
        assert!(matches!(
            mailbox.take().as_slice(),
            [ViewerOutput::Status(_)]
        ));
    }

    #[test]
    fn vertical_position_failure_coalesces_before_the_frame_without_spending_its_capacity() {
        for failure_first in [false, true] {
            let mailbox = Mailbox::with_layout();
            if failure_first {
                mailbox.put(ViewerOutput::VerticalPositionUnavailable {
                    generation: 7,
                    sequence: 8,
                });
            }
            for _ in 0..100 {
                mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
            }
            mailbox.put(ViewerOutput::LayoutSelected {
                token: 19,
                mode: VIEWER_MODE_HTML,
                content_generation: 7,
            });
            mailbox.put(ViewerOutput::PaletteApplied {
                revision: 5,
                generation: 7,
                minimum_frame_revision: Some(1),
            });
            mailbox.put(ViewerOutput::RasterRebase {
                surface: ViewerSurfaceId(3),
                transition: 41,
            });
            mailbox.put(frame());
            for (generation, sequence) in [(7, 12), (6, 99), (7, 11), (7, 12)] {
                mailbox.put(ViewerOutput::VerticalPositionUnavailable {
                    generation,
                    sequence,
                });
            }

            let outputs = mailbox.take();
            assert_eq!(outputs.len(), 9);
            assert_eq!(
                outputs
                    .iter()
                    .filter(|output| matches!(
                        output,
                        ViewerOutput::WebShortcut(HostShortcut::Find)
                    ))
                    .count(),
                4
            );
            assert!(matches!(
                outputs[0],
                ViewerOutput::LayoutSelected { token: 19, .. }
            ));
            assert!(matches!(
                outputs[1],
                ViewerOutput::PaletteApplied { revision: 5, .. }
            ));
            assert!(matches!(
                outputs[6],
                ViewerOutput::RasterRebase { transition: 41, .. }
            ));
            assert!(matches!(
                outputs[7],
                ViewerOutput::VerticalPositionUnavailable {
                    generation: 7,
                    sequence: 12
                }
            ));
            assert!(matches!(outputs[8], ViewerOutput::Frame(_)));
        }
    }

    #[test]
    fn vertical_position_failure_duplicate_can_retry_a_rejected_wake() {
        let mailbox = Mailbox::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        mailbox.notify_on_ready(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed) != 0
        }));
        for expected_calls in 1..=2 {
            mailbox.put(ViewerOutput::VerticalPositionUnavailable {
                generation: 7,
                sequence: 8,
            });
            assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
        }
        assert!(matches!(
            mailbox.take().as_slice(),
            [ViewerOutput::VerticalPositionUnavailable {
                generation: 7,
                sequence: 8
            }]
        ));
    }

    #[test]
    fn closed_retires_the_pending_vertical_position_failure_and_fences_late_failure() {
        let mailbox = Mailbox::default();
        mailbox.put(ViewerOutput::VerticalPositionUnavailable {
            generation: 7,
            sequence: 8,
        });
        mailbox.put(ViewerOutput::Closed);
        mailbox.put(ViewerOutput::VerticalPositionUnavailable {
            generation: 7,
            sequence: 9,
        });
        assert!(matches!(mailbox.take().as_slice(), [ViewerOutput::Closed]));
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
        assert_eq!(outputs.len(), 7);
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
            4
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
        assert_eq!(outputs.len(), 7);
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
    fn fixed_viewport_mailbox_reserves_palette_and_keeps_five_ordinary_events() {
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
        assert_eq!(outputs.len(), 7);
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

#[cfg(test)]
#[path = "mailbox_palette_tests.rs"]
mod palette_tests;

/// The text a (whole-file) Copy produced and whether one runs reach the window however
/// busy it is, the latest of each only.
#[cfg(test)]
#[test]
fn copied_text_and_the_copy_state_survive_full_event_pressure() {
    let mailbox = Mailbox::default();
    mailbox.put(ViewerOutput::Copying(true));
    for _ in 0..100 {
        mailbox.put(ViewerOutput::WebShortcut(
            midnight_core::viewer_web::HostShortcut::Find,
        ));
    }
    mailbox.put(ViewerOutput::CopyText("whole file".into()));
    mailbox.put(ViewerOutput::Copying(false));
    let output = mailbox.take();
    let copies: Vec<_> = output
        .iter()
        .filter_map(|event| match event {
            ViewerOutput::CopyText(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(copies, ["whole file"]);
    let states: Vec<_> = output
        .iter()
        .filter_map(|event| match event {
            ViewerOutput::Copying(running) => Some(*running),
            _ => None,
        })
        .collect();
    assert_eq!(states, [false], "the latest state replaces the earlier one");
}
