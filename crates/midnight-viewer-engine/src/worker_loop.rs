//! Event-driven worker loop plumbing: wakes, bounded input batches and file identities.
use super::*;
use std::time::Instant;

/// How long the worker polls for the end of a session after an orphaned failure, because
/// the end of a session has no wake signal.
const ORPHAN_WATCH: Duration = Duration::from_secs(1);
/// Events handled per drain of the active session.
const DRAIN: usize = 32;

/// The leading INVALID failure of a drain that the worker does not show as its error, such
/// as a late failure of a cancelled request or a failure of the optional locate request.
/// Plugin-host drains a session-fatal failure before any other event and pushes nothing
/// after it, so this failure ended the session when the session ends before delivering
/// another event.
pub(super) struct Orphan {
    pub(super) message: String,
    /// The worker polls for the session's end until then.
    pub(super) watch: Instant,
}

impl Worker {
    /// `None` blocks until a wake or input arrives. Otherwise retry after the delay.
    pub(super) fn retry_delay(&self) -> Option<Duration> {
        if self.web_backlog {
            return Some(Duration::ZERO);
        }
        let closing = self.pending_open && !self.closings_finished();
        let orphan = self.orphan_failure.as_ref().is_some_and(|orphan| {
            Instant::now() < orphan.watch
                && self
                    .session
                    .as_ref()
                    .is_some_and(|session| !session.is_closed())
        });
        (self.dirty || closing || orphan).then_some(RETRY)
    }

    pub(super) fn closings_finished(&self) -> bool {
        self.closing.as_ref().is_none_or(Session::is_closed)
            && self
                .closing_web
                .as_ref()
                .is_none_or(midnight_web_viewer_host::Session::finished)
            && self
                .closing_image
                .as_ref()
                .is_none_or(crate::image_worker::ImageWorker::finished)
    }

    /// Apply one bounded batch of channel messages. Only the last of consecutive resizes
    /// is applied: it is compared with the geometry before the run, so its layout and
    /// refill decisions cover the skipped ones.
    pub(super) fn input_batch(
        &mut self,
        batch: Vec<Message>,
        closed: &AtomicBool,
        wakes: &crate::input_queue::Wakes,
        output: &Mailbox,
    ) {
        let mut superseded = vec![false; batch.len()];
        let mut resize_follows = false;
        for (index, message) in batch.iter().enumerate().rev() {
            match message {
                Message::Ready => {}
                Message::Input(ViewerInput::Resize(_)) => {
                    superseded[index] = resize_follows;
                    resize_follows = true;
                }
                _ => resize_follows = false,
            }
        }
        for (message, superseded) in batch.into_iter().zip(superseded) {
            if closed.load(Ordering::Acquire) || self.layout.closed() {
                return;
            }
            if matches!(message, Message::Ready) {
                wakes.received();
            } else if !superseded && let Some(input) = message.into_input() {
                self.input(input, output);
            }
        }
    }

    pub(super) fn capture_ready(&self) -> bool {
        matches!(&self.captured, Some((index, _)) if *index == self.current)
    }

    /// Ask for the current file's identity unless it is held or already requested.
    pub(super) fn request_capture(&mut self) {
        if self.capture_ready()
            || self
                .capture_request
                .is_some_and(|(_, index)| index == self.current)
        {
            return;
        }
        self.captured = None;
        match &self.prefetch {
            Some(prefetch) => {
                self.capture_request = Some((prefetch.request(self.current), self.current));
                self.poll_capture();
            }
            None => {
                self.captured = Some((
                    self.current,
                    Err("File identity is unavailable for this viewer.".into()),
                ));
            }
        }
    }

    pub(super) fn poll_capture(&mut self) {
        if let (Some((sequence, index)), Some(prefetch)) = (self.capture_request, &self.prefetch)
            && let Some(result) = prefetch.take(sequence)
        {
            self.capture_request = None;
            if index == self.current {
                self.captured = Some((index, result));
            }
        }
    }

    /// Continue a deferred opening once its identity is ready and every previous session,
    /// browser and image actor has finished closing. The opening requested the identity
    /// when it began, so the capture overlaps that teardown.
    pub(super) fn resume_open(&mut self, output: &Mailbox) {
        self.poll_capture();
        if !self.pending_open || !self.closings_finished() {
            return;
        }
        if self.capture_ready() {
            self.closing = None;
            self.open(output);
        } else if self.status != PREPARING {
            self.status(PREPARING.into(), output);
        }
    }

    /// Whether a Failed event with this ID is shown as the viewer's error.
    pub(super) fn reports_failure(&self, id: u64) -> bool {
        id == OPEN_REQUEST_ID
            || self.search_request == Some(id)
            || self.viewport_request == Some(id)
    }

    /// Handle one bounded drain of the active session, then an end the worker did not cause.
    pub(super) fn drain_session(&mut self, output: &Mailbox) {
        let Some(session) = &self.session else {
            return;
        };
        // Read before draining: a closed session pushes nothing more, so this drain
        // delivers its last events, including a pending session-fatal failure.
        let ended = session.is_closed();
        let events = session.drain(DRAIN);
        self.session_events(events, ended, output);
    }

    /// `ended`: the session had stopped before `events` were drained from it.
    pub(super) fn session_events(
        &mut self,
        events: Vec<midnight_plugin_host::HostEvent>,
        ended: bool,
        output: &Mailbox,
    ) {
        if !events.is_empty() {
            // A session pushes nothing after its fatal failure, so a later event shows that
            // an earlier drain's orphan was not that failure.
            self.orphan_failure = None;
        }
        for (index, event) in events.into_iter().enumerate() {
            if event.generation != self.generation {
                continue;
            }
            // Plugin-host reports a session-fatal failure as INVALID, with the offending
            // request's ID, ahead of older queued events. A late failure of a cancelled
            // request elsewhere in a drain is not shown.
            let orphan = match &event.kind {
                EventKind::Failed(failure)
                    if index == 0
                        && failure.code == midnight_plugin_host::api::abi::INVALID
                        && !self.reports_failure(event.request_id) =>
                {
                    Some(failure.message.clone())
                }
                _ => None,
            };
            self.event(event.request_id, event.kind, output);
            if let Some(message) = orphan {
                self.orphan_failed(message);
            }
        }
        self.report_session_end(ended, output);
    }

    pub(super) fn orphan_failed(&mut self, message: String) {
        self.orphan_failure = Some(Orphan {
            message,
            watch: Instant::now() + ORPHAN_WATCH,
        });
    }

    /// `ended`: the active session stopped although this worker did not close it, which
    /// follows a fatal failure. Report an orphaned fatal failure instead of a later generic
    /// "closing" error. A tracked fatal failure has already been reported.
    pub(super) fn report_session_end(&mut self, ended: bool, output: &Mailbox) {
        if ended && let Some(orphan) = self.orphan_failure.take() {
            self.session = None;
            self.search_request = None;
            self.queued_scroll = [0.0; 2];
            self.continuation = None;
            self.position_context(output);
            self.viewport_request = None;
            self.error(orphan.message, output);
        }
    }
}
