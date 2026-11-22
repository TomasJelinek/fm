//! Event-driven worker loop plumbing: wakes, bounded input batches and file identities.
use super::*;

impl Worker {
    /// `None` blocks until a wake or input arrives. Otherwise retry after the delay.
    pub(super) fn retry_delay(&self) -> Option<Duration> {
        if self.web_backlog || self.raster.strips_pending() {
            return Some(Duration::ZERO);
        }
        // A pending opening needs no retry: a closing plugin session signals the end of its
        // dispatcher (see `Worker::closing`), and closing browser and image actors wake this
        // worker once they finished (see `actors_finished`).
        let deadline = self
            .source_retry_deadline()
            .into_iter()
            .chain(self.locate_retry_deadline())
            .min();
        self.dirty
            .then_some(RETRY)
            .into_iter()
            .chain(deadline.map(|at| at.saturating_duration_since(std::time::Instant::now())))
            .chain(self.video.as_ref().and_then(VideoView::delay))
            .min()
    }

    pub(super) fn closings_finished(&self) -> bool {
        self.closing.as_ref().is_none_or(Session::is_closed) && self.actors_finished()
    }

    /// Previous browser and image actors have finished closing. Each actor thread publishes
    /// its end, then wakes this worker.
    fn actors_finished(&self) -> bool {
        self.closing_web
            .as_ref()
            .is_none_or(|session| session.finished())
            && self
                .closing_image
                .as_ref()
                .is_none_or(crate::image_worker::ImageWorker::finished)
            && self.closing_video.as_ref().is_none_or(VideoView::finished)
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
            } else if let Message::Scroll(step) = message {
                self.admitted_scroll(step, output);
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

    pub(super) fn video_input(&mut self, input: &ViewerInput, output: &Mailbox) -> bool {
        match input {
            ViewerInput::NextFile | ViewerInput::PreviousFile | ViewerInput::Mode(_) => false,
            ViewerInput::Resize(geometry) => {
                self.geometry = bounded_geometry(*geometry);
                self.image_size = [geometry.width.max(1), geometry.height.max(1)];
                true
            }
            ViewerInput::MediaControl(control) => {
                let Some(video) = &mut self.video else {
                    return true;
                };
                match video.apply_control(*control) {
                    Ok(true) => {
                        if matches!(control.action, MediaAction::Seek(_)) {
                            output.discard_pending_video();
                        }
                        self.poll_video(output);
                    }
                    Ok(false) => {}
                    Err(error) => self.fail_video(error, output),
                }
                true
            }
            _ => true,
        }
    }

    fn cancel_video(&mut self) {
        if let Some(mut video) = self.video.take() {
            video.cancel();
            self.closing_video = Some(video);
        }
    }

    pub(super) fn retire_video(&mut self) -> Result<bool, String> {
        self.cancel_video();
        if self
            .closing_video
            .as_ref()
            .is_some_and(|video| !video.finished())
        {
            return Ok(false);
        }
        self.finish_video().map(|()| true)
    }

    pub(super) fn finish_video(&mut self) -> Result<(), String> {
        self.cancel_video();
        if let Some(video) = self.closing_video.take()
            && let Err(error) = video.finish()
        {
            self.video_retirement_error.get_or_insert(error);
        }
        self.video_retirement_error.clone().map_or(Ok(()), Err)
    }

    pub(super) fn poll_video(&mut self, output: &Mailbox) {
        let Some(video) = &mut self.video else {
            if !self.pending_open
                && self.closing_video.as_ref().is_some_and(VideoView::finished)
                && let Err(error) = self.finish_video()
            {
                self.error(error, output);
            }
            return;
        };
        let result = match video.poll() {
            Ok(Some(update)) => self.publish_video(update, output),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.fail_video(error, output);
        }
    }

    fn fail_video(&mut self, error: String, output: &Mailbox) {
        let snapshot = self.video.as_mut().map(|video| video.fail(error.clone()));
        output.discard_pending_video();
        self.error(error, output);
        if let Some(snapshot) = snapshot {
            output.put(ViewerOutput::MediaSnapshot(snapshot));
        }
    }

    pub(super) fn publish_video(
        &mut self,
        update: VideoUpdate,
        output: &Mailbox,
    ) -> Result<(), String> {
        let VideoUpdate {
            frame,
            snapshot,
            notice,
        } = update;
        if let Some(notice) = notice {
            self.status(notice, output);
        } else if frame.is_some() && self.status == "Opening…" {
            self.status(String::new(), output);
        }
        let generation = snapshot.session.0;
        if let Some(mut frame) = frame {
            for pixel in frame.rgba.as_chunks_mut::<4>().0 {
                let alpha = u16::from(pixel[3]);
                if alpha != 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                    }
                }
            }
            output.put(ViewerOutput::Frame(Box::new(ViewerFrame {
                generation,
                media_control: Some(snapshot.control),
                position_sequence: [0; 2],
                revision: self.frames.painted()?,
                mode: VIEWER_MODE_VIDEO,
                modes: vec![VIEWER_MODE_VIDEO],
                wrap_enabled: false,
                wrap_available: false,
                image: None,
                raster: None,
                raster_rebase: None,
                width: frame.width,
                height: frame.height,
                pixels: frame.rgba.into(),
                tiles: None,
                accessible_text: Arc::from("Video"),
                position: 0.0,
                vertical_extent: 1.0,
                horizontal_position: 0.0,
                horizontal_extent: 1.0,
                caption: self.caption.clone(),
                status: self.status.clone(),
            })));
        }
        output.put(ViewerOutput::MediaSnapshot(snapshot));
        Ok(())
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
            for (id, kind) in self.chain_event(event.request_id, event.kind) {
                self.event(id, kind, output);
            }
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
        self.chains.clear();
        if let Some(id) = self.viewport_request {
            self.viewport_failed(id, &message, output);
        }
        self.search_request = None;
        self.search_cursor.cancel_candidate();
        self.text.locate_request = None;
        self.motion = RelativeMotion::default();
        self.continuation = None;
        self.position_context(output);
        self.error(message, output);
    }
}
