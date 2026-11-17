//! Event-driven worker loop plumbing: wakes, bounded input batches and file identities.
use super::*;

/// Watch a session for this long after a failure of a request the worker had cancelled.
/// Plugin-host reports a session-fatal failure with the offending request ID, and the
/// session's teardown has no wake signal.
const ORPHAN_WATCH: Duration = Duration::from_secs(1);

impl Worker {
    /// `None` blocks until a wake or input arrives. Otherwise retry after the delay.
    pub(super) fn retry_delay(&self) -> Option<Duration> {
        if self.web_backlog {
            return Some(Duration::ZERO);
        }
        let closing = self.pending_open && !self.closings_finished();
        let orphan = self.orphan_failure.as_ref().is_some_and(|(_, watch)| {
            std::time::Instant::now() < *watch
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

    /// A failed request ID the worker no longer tracks, outside the Open request.
    pub(super) fn orphan_failed(&mut self, message: String) {
        self.orphan_failure = Some((message, std::time::Instant::now() + ORPHAN_WATCH));
    }

    /// `ended`: the active session stopped although this worker did not close it, which
    /// follows a fatal failure. Report the failure that preceded the end instead of a later
    /// generic "closing" error.
    pub(super) fn report_session_end(&mut self, ended: bool, output: &Mailbox) {
        if ended && let Some((message, _)) = self.orphan_failure.take() {
            self.session = None;
            self.search_request = None;
            self.queued_scroll = [0.0; 2];
            self.continuation = None;
            self.position_context(output);
            self.viewport_request = None;
            self.error(message, output);
        }
    }
}
