//! Bounded handoff between the viewer's playback/output thread and its decoder broker.
use midnight_core::{
    MediaAction, MediaControl, MediaControlId, MediaPhase, MediaSessionId, MediaSnapshot,
};
use midnight_media_host::{
    CpuBudget, PlaybackEpoch, PlaybackSnapshot, PlaybackState, ResourcePolicy,
    video_installation::VideoInstallation,
    video_process::{CapturedVideoSource, VideoProcess},
    video_session::{DecoderReply, DecoderRequest, VideoSession},
    video_transport::{
        OutputBounds, SessionGeneration, VideoFrame, VideoIdentity, VideoIo, VideoMetadata,
        VideoTransport,
    },
};
use midnight_plugin_host::CapturedFile;
use std::{
    io,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const STARTUP: Duration = Duration::from_secs(12);
const READ: Duration = Duration::from_secs(8);
const RETIRE: Duration = Duration::from_secs(4);
const TICK: Duration = Duration::from_millis(12);
const MAX_DECODERS: usize = 4;
static DECODERS: AtomicUsize = AtomicUsize::new(0);

struct DecoderPermit;

impl DecoderPermit {
    fn acquire(cancel: &AtomicBool, deadline: Instant) -> io::Result<Self> {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Video decoder capacity did not become available",
                    )
                })?;
            if DECODERS
                .try_update(Ordering::AcqRel, Ordering::Acquire, |running| {
                    (running < MAX_DECODERS).then_some(running + 1)
                })
                .is_ok()
            {
                return Ok(Self);
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

impl Drop for DecoderPermit {
    fn drop(&mut self) {
        DECODERS.fetch_sub(1, Ordering::AcqRel);
    }
}

enum Reply {
    Opened(VideoIdentity, VideoMetadata),
    Finished(DecoderReply),
    Failed(String),
}

pub(crate) struct VideoUpdate {
    pub frame: Option<VideoFrame>,
    pub snapshot: MediaSnapshot,
    pub notice: Option<String>,
}

pub(crate) fn opening(session: MediaSessionId) -> MediaSnapshot {
    MediaSnapshot {
        session,
        control: MediaControlId(0),
        revision: 1,
        position: Duration::ZERO,
        duration: None,
        seekable: false,
        playing: false,
        phase: MediaPhase::Opening,
    }
}

#[derive(Default)]
struct ControlProgress {
    accepted: MediaControlId,
    pending_seek: Option<(MediaControlId, PlaybackEpoch)>,
    pending_play: Option<MediaControlId>,
}

impl ControlProgress {
    fn admit(&mut self, id: MediaControlId) -> bool {
        if self.accepted.0.checked_add(1) != Some(id.0) {
            return false;
        }
        self.accepted = id;
        true
    }

    fn seek(&mut self, id: MediaControlId, epoch: PlaybackEpoch) {
        self.pending_seek = Some((id, epoch));
        self.pending_play = None;
    }

    fn settle(&mut self, playback: PlaybackSnapshot, snapshot: &mut MediaSnapshot) {
        if playback.state == PlaybackState::Seeking {
            return;
        }
        if let Some((id, epoch)) = self.pending_seek {
            if epoch != playback.epoch {
                return;
            }
            snapshot.control = id;
            self.pending_seek = None;
        }
        // A later play/pause intent settles only after the earlier seek.
        if let Some(id) = self.pending_play.take() {
            snapshot.control = id;
        }
        snapshot.position = snapshot
            .duration
            .map_or(playback.position, |end| playback.position.min(end));
    }

    fn fail(&mut self, snapshot: &mut MediaSnapshot, message: String) {
        self.pending_seek = None;
        self.pending_play = None;
        snapshot.control = self.accepted;
        snapshot.playing = false;
        snapshot.seekable = false;
        snapshot.phase = MediaPhase::Failed(message);
    }
}

pub(crate) struct VideoView {
    cancelled: Arc<AtomicBool>,
    command: Option<SyncSender<DecoderRequest>>,
    replies: Receiver<Reply>,
    broker: Option<JoinHandle<io::Result<()>>>,
    decoder_finished: Arc<AtomicBool>,
    session: Option<VideoSession>,
    duration: Option<Duration>,
    wants_play: bool,
    controls: ControlProgress,
    presentation: MediaSnapshot,
    notice: Option<String>,
    retirement_error: Option<io::Error>,
}

impl VideoView {
    #[cfg(test)]
    pub(crate) fn finished_for_test(session: MediaSessionId) -> Self {
        let (_, replies) = mpsc::sync_channel(2);
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            command: None,
            replies,
            broker: None,
            decoder_finished: Arc::new(AtomicBool::new(true)),
            session: None,
            duration: None,
            wants_play: false,
            controls: ControlProgress::default(),
            presentation: opening(session),
            notice: None,
            retirement_error: None,
        }
    }

    pub(crate) fn start(
        installation: VideoInstallation,
        file: CapturedFile,
        generation: u64,
        width: u32,
        height: u32,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, String> {
        let id = NonZeroU64::new(generation).ok_or("Invalid video session identity")?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let decoder_finished = Arc::new(AtomicBool::new(false));
        let finished = decoder_finished.clone();
        let (command, incoming) = mpsc::sync_channel(1);
        let (results, replies) = mpsc::sync_channel(2);
        let broker = thread::Builder::new()
            .name("video-decoder-broker".into())
            .spawn(move || {
                let _finished = NotifyFinished {
                    finished,
                    wake: wake.clone(),
                };
                let publish = |reply| {
                    // One request plus one error fit the bounded reply channel.
                    results
                        .try_send(reply)
                        .map_err(|_| io::Error::other("Video viewer closed"))?;
                    wake();
                    Ok::<(), io::Error>(())
                };
                let bounds = OutputBounds {
                    width: width.clamp(1, 1920),
                    height: height.clamp(1, 1080),
                };
                let result = run(
                    installation,
                    file,
                    SessionGeneration(id),
                    bounds,
                    incoming,
                    &stop,
                    &publish,
                );
                if let Err(error) = &result.playback
                    && !stop.load(Ordering::Acquire)
                {
                    let _ = publish(Reply::Failed(error.to_string()));
                }
                result.retired
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancelled,
            command: Some(command),
            replies,
            broker: Some(broker),
            decoder_finished,
            session: None,
            duration: None,
            wants_play: false,
            controls: ControlProgress::default(),
            presentation: opening(MediaSessionId(generation)),
            notice: None,
            retirement_error: None,
        })
    }

    pub(crate) fn presentation(&self) -> MediaSnapshot {
        self.presentation.clone()
    }

    pub(crate) fn session_id(&self) -> MediaSessionId {
        self.presentation.session
    }

    pub(crate) fn fail(&mut self, message: String) -> MediaSnapshot {
        self.cancel();
        self.wants_play = false;
        self.controls.fail(&mut self.presentation, message);
        self.presentation.revision = self.presentation.revision.saturating_add(1);
        self.presentation()
    }

    pub(crate) fn poll(&mut self) -> Result<Option<VideoUpdate>, String> {
        if matches!(self.presentation.phase, MediaPhase::Failed(_)) {
            if self.finished()
                && let Err(error) = self.join()
            {
                self.retirement_error.get_or_insert(error);
            }
            if self.presentation.control == self.controls.accepted {
                return Ok(None);
            }
            self.presentation.control = self.controls.accepted;
            self.presentation.revision = self.presentation.revision.saturating_add(1);
            return Ok(Some(VideoUpdate {
                frame: None,
                snapshot: self.presentation(),
                notice: None,
            }));
        }
        let now = Instant::now();
        while let Ok(reply) = self.replies.try_recv() {
            match reply {
                Reply::Opened(identity, metadata) => {
                    self.duration = metadata.duration;
                    let mut session =
                        VideoSession::new(identity, metadata, midnight_media_output::open_default);
                    self.notice = session.audio_notice().map(str::to_owned);
                    if self.wants_play {
                        session.play(now).map_err(|error| error.to_string())?;
                    }
                    self.session = Some(session);
                }
                Reply::Finished(reply) => self
                    .session
                    .as_mut()
                    .ok_or("Video failed to open")?
                    .accept(reply, now)
                    .map_err(|error| error.to_string())?,
                Reply::Failed(error) => return Err(error),
            }
        }
        if self.finished() && !self.cancelled.load(Ordering::Acquire) {
            return Err("Video decoder exited before the current session closed".into());
        }
        let Some(session) = self.session.as_mut() else {
            return Ok(None);
        };
        let tick = session
            .tick(now, &self.cancelled)
            .map_err(|error| error.to_string())?;
        let awaiting_first_frame =
            matches!(self.presentation.phase, MediaPhase::Opening) && tick.frame.is_none();
        if tick.finished {
            if awaiting_first_frame {
                return Err("Video contains no decodable frames".into());
            }
            self.wants_play = false;
        }
        if let Some(request) = session.next_request() {
            self.command
                .as_ref()
                .ok_or("Video closed")?
                .try_send(request)
                .map_err(|_| "Video decoder is unavailable".to_string())?;
        }
        let mut snapshot = self.presentation();
        snapshot.playing = self.wants_play;
        snapshot.duration = self.duration;
        snapshot.seekable = self.duration.is_some_and(|end| !end.is_zero());
        snapshot.phase = match tick.playback.state {
            _ if tick.finished => MediaPhase::Ended,
            _ if awaiting_first_frame => MediaPhase::Opening,
            PlaybackState::Seeking => MediaPhase::Seeking,
            PlaybackState::Paused => MediaPhase::Paused,
            PlaybackState::Playing if tick.buffering => MediaPhase::Buffering,
            PlaybackState::Playing => MediaPhase::Playing,
            PlaybackState::Faulted | PlaybackState::Closed => {
                return Err("Video playback has stopped".into());
            }
        };
        self.controls.settle(tick.playback, &mut snapshot);
        if tick.frame.is_none() && snapshot == self.presentation && self.notice.is_none() {
            return Ok(None);
        }
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or("Video presentation revision exhausted")?;
        self.presentation = snapshot.clone();
        Ok(Some(VideoUpdate {
            frame: tick.frame,
            snapshot,
            notice: self.notice.take(),
        }))
    }

    pub(crate) fn apply_control(&mut self, control: MediaControl) -> Result<bool, String> {
        if control.session != self.presentation.session || !self.controls.admit(control.id) {
            return Ok(false);
        }
        // A command can be in the viewer queue when the decoder reports a
        // terminal failure. Let poll publish that command's failed settlement.
        if matches!(self.presentation.phase, MediaPhase::Failed(_)) {
            return Ok(true);
        }
        let session = self.session.as_mut().ok_or("Video has not opened")?;
        let now = Instant::now();
        let result = match control.action {
            MediaAction::SetPlaying(playing) => {
                self.wants_play = playing;
                self.controls.pending_play = Some(control.id);
                if playing {
                    session.play(now)
                } else {
                    session.pause(now)
                }
            }
            MediaAction::Seek(position) => {
                if self.duration.is_none_or(|end| end.is_zero()) {
                    return Err("This video is not seekable".into());
                }
                session.seek(position).map(|epoch| {
                    self.controls.seek(control.id, epoch);
                })
            }
        };
        result.map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub(crate) fn delay(&self) -> Option<Duration> {
        matches!(
            self.presentation.phase,
            MediaPhase::Playing | MediaPhase::Buffering
        )
        .then_some(TICK)
    }

    pub(crate) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.command.take();
        // Detach audible output before waiting for a previous source reader or
        // the restricted process to retire. These calls stay on the same worker.
        if let Some(mut session) = self.session.take() {
            self.retirement_error = session.close().err();
        }
    }

    pub(crate) fn finished(&self) -> bool {
        self.decoder_finished.load(Ordering::Acquire)
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.cancel();
        let decoder = self.join();
        self.retirement_error
            .take()
            .map_or(decoder, Err)
            .map_err(|error| error.to_string())
    }

    fn join(&mut self) -> io::Result<()> {
        let Some(broker) = self.broker.take() else {
            return Ok(());
        };
        // Protocol and source failures are published before the broker exits.
        // A separate close reports only audio retirement or broker teardown.
        broker.join().map_err(|_| {
            io::Error::other("Video decoder broker stopped without confirming retirement")
        })?
    }
}

impl Drop for VideoView {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.join();
    }
}

struct NotifyFinished {
    finished: Arc<AtomicBool>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Drop for NotifyFinished {
    fn drop(&mut self) {
        // The receiver can consume this wake before the OS joins the thread.
        // Publish the completed cleanup transition before waking it.
        self.finished.store(true, Ordering::Release);
        (self.wake)();
    }
}

struct BrokerResult {
    playback: io::Result<()>,
    retired: io::Result<()>,
}

fn run(
    installation: VideoInstallation,
    file: CapturedFile,
    session: SessionGeneration,
    bounds: OutputBounds,
    incoming: Receiver<DecoderRequest>,
    cancel: &Arc<AtomicBool>,
    publish: &impl Fn(Reply) -> io::Result<()>,
) -> BrokerResult {
    let deadline = Instant::now() + STARTUP;
    let mut retirement = None;
    let playback = (|| {
        let permit = DecoderPermit::acquire(cancel, deadline)?;
        // The owner and the transport share one consuming capture wrapper.
        let source = CapturedVideoSource::new(file, false);
        let process = VideoProcess::launch(
            &installation,
            ResourcePolicy {
                memory_bytes: 512 * 1024 * 1024,
                cpu: CpuBudget::NoAdditionalLimit,
            },
            cancel.clone(),
            deadline,
            (source.clone(), permit),
        )?;
        retirement = Some(process.retirement());
        let mut transport = VideoTransport::open(
            process,
            source,
            session,
            bounds,
            VideoIo {
                deadline,
                cancelled: cancel,
            },
        )?;
        publish(Reply::Opened(transport.identity(), transport.metadata()))?;
        while let Ok(request) = incoming.recv() {
            if cancel.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let io = VideoIo {
                deadline: Instant::now() + READ,
                cancelled: cancel,
            };
            let reply = match request {
                DecoderRequest::Pull(credit) => DecoderReply::Pulled(transport.pull(credit, io)?),
                DecoderRequest::Seek(position) => {
                    transport.seek(position, io)?;
                    DecoderReply::Sought(transport.identity())
                }
            };
            publish(Reply::Finished(reply))?;
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        transport.close(VideoIo {
            deadline: Instant::now() + READ,
            cancelled: cancel,
        })
    })();
    // Transport and both pipe ends have been retired before waiting here.
    // The receipt keeps source and capacity owned until the actual OS result.
    let retired = retirement.map_or(Ok(()), |finished| finished.wait(Instant::now() + RETIRE));
    BrokerResult { playback, retired }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_media_host::Playback;

    #[test]
    fn a_newer_play_intent_cannot_acknowledge_an_unsettled_seek() {
        let mut playback = Playback::video_only(Duration::ZERO);
        let mut progress = ControlProgress::default();
        let seek = playback.seek(Duration::from_secs(1)).unwrap();
        progress.admit(MediaControlId(1));
        progress.seek(MediaControlId(1), seek.epoch);
        progress.admit(MediaControlId(2));
        progress.pending_play = Some(MediaControlId(2));

        let mut snapshot = opening(MediaSessionId(4));
        progress.settle(playback.snapshot(), &mut snapshot);
        assert_eq!(snapshot.control, MediaControlId(0));

        assert!(
            playback
                .acknowledge_seek(seek.epoch, Instant::now())
                .unwrap()
        );
        progress.settle(playback.snapshot(), &mut snapshot);
        assert_eq!(snapshot.control, MediaControlId(2));
    }

    #[test]
    fn replacement_seek_only_settles_after_its_own_epoch() {
        let mut playback = Playback::video_only(Duration::ZERO);
        let mut progress = ControlProgress::default();
        progress.admit(MediaControlId(1));
        let first = playback.seek(Duration::from_secs(1)).unwrap();
        progress.seek(MediaControlId(1), first.epoch);
        progress.admit(MediaControlId(2));
        let last = playback.seek(Duration::from_secs(2)).unwrap();
        progress.seek(MediaControlId(2), last.epoch);
        let mut snapshot = opening(MediaSessionId(5));

        assert!(
            !playback
                .acknowledge_seek(first.epoch, Instant::now())
                .unwrap()
        );
        progress.settle(playback.snapshot(), &mut snapshot);
        assert_eq!(snapshot.control, MediaControlId(0));
        assert!(
            playback
                .acknowledge_seek(last.epoch, Instant::now())
                .unwrap()
        );
        progress.settle(playback.snapshot(), &mut snapshot);
        assert_eq!(snapshot.control, MediaControlId(2));
        assert_eq!(snapshot.position, Duration::from_secs(2));

        progress.admit(MediaControlId(3));
        let failed = playback.seek(Duration::from_secs(3)).unwrap();
        progress.seek(MediaControlId(3), failed.epoch);
        progress.fail(&mut snapshot, "source unavailable".into());
        assert_eq!(snapshot.control, MediaControlId(3));
        assert_eq!(snapshot.position, Duration::from_secs(2));
        assert_eq!(
            snapshot.phase,
            MediaPhase::Failed("source unavailable".into())
        );
    }
}
