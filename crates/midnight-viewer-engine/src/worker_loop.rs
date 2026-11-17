//! Event-driven worker loop plumbing: wakes, bounded input batches and file identities.
use super::*;

impl Worker {
    /// `None` blocks until a wake or input arrives. Otherwise retry after the delay.
    pub(super) fn retry_delay(&self) -> Option<Duration> {
        if self.web_backlog {
            return Some(Duration::ZERO);
        }
        // A closing plugin session signals the end of its dispatcher (see `Worker::closing`),
        // so only browser and image teardown is retried.
        let closing = self.pending_open && !self.actors_finished();
        (self.dirty || closing).then_some(RETRY)
    }

    pub(super) fn closings_finished(&self) -> bool {
        self.closing.as_ref().is_none_or(Session::is_closed) && self.actors_finished()
    }

    /// Previous browser and image actors have finished closing. Their end has no wake.
    fn actors_finished(&self) -> bool {
        self.closing_web
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

    /// Handle one bounded drain of the active session.
    pub(super) fn drain_session(&mut self, output: &Mailbox) {
        let Some(session) = &self.session else {
            return;
        };
        let events = session.drain(DRAIN);
        self.session_events(events, output);
    }

    /// Apply drained events in order until a session-fatal failure. Plugin-host drains
    /// that failure ahead of every older event it still holds, and the session is
    /// retired with it, so neither the rest of this drain nor any later one applies.
    pub(super) fn session_events(
        &mut self,
        events: Vec<midnight_plugin_host::HostEvent>,
        output: &Mailbox,
    ) {
        for event in events {
            if event.generation != self.generation {
                continue;
            }
            if event.fatal {
                let message = match event.kind {
                    EventKind::Failed(failure) => failure.message,
                    _ => "The viewer plugin stopped.".into(),
                };
                self.session_failed(message, output);
                return;
            }
            self.event(event.request_id, event.kind, output);
        }
    }

    /// The active session failed fatally: plugin-host already stopped it, answers none of
    /// its outstanding requests and refuses new ones with "Viewer is closing". Show this
    /// failure at once, whichever request it names (a cancelled one or the optional locate
    /// request included), end what waited on the session, and retire the session as a
    /// closing one: the next opening still waits for its teardown, and later actions find
    /// no session instead of replacing the message with that refusal.
    fn session_failed(&mut self, message: String, output: &Mailbox) {
        if let Some(session) = self.session.take() {
            self.closing = Some(session);
        }
        if let Some(id) = self.viewport_request {
            self.viewport_failed(id, &message, output);
        }
        self.search_request = None;
        self.text.locate_request = None;
        self.queued_scroll = [0.0; 2];
        self.continuation = None;
        self.position_context(output);
        self.error(message, output);
    }
}
