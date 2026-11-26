//! Bounded handoff between the viewer's playback/output thread and its decoder broker.
use midnight_core::{
    MediaAction, MediaControl, MediaControlId, MediaPhase, MediaSessionId, MediaSnapshot,
};
use midnight_media_host::{
    AudioSink, AudioVolume, CpuBudget, PlaybackEpoch, PlaybackSnapshot, PlaybackState,
    ResourcePolicy,
    video_installation::{VideoInstallation, VideoWorker, both_failed, fallback_worker},
    video_process::{CapturedVideoSource, VideoProcess, VideoRetirement},
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
        audio: false,
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

/// Opens the native output for a video with a qualified audio track.
pub(crate) type OpenAudio = fn() -> io::Result<Box<dyn AudioSink>>;

pub(crate) struct VideoView {
    cancelled: Arc<AtomicBool>,
    command: Option<SyncSender<DecoderRequest>>,
    /// Gives the broker the canvas size it waits for before opening the video.
    size: Option<SyncSender<OutputBounds>>,
    replies: Receiver<Reply>,
    broker: Option<JoinHandle<io::Result<()>>>,
    decoder_finished: Arc<AtomicBool>,
    session: Option<VideoSession>,
    /// The window's sound level, shared with the playback of every video it opens.
    volume: Arc<AudioVolume>,
    open_audio: OpenAudio,
    audio: bool,
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
            size: None,
            replies,
            broker: None,
            decoder_finished: Arc::new(AtomicBool::new(true)),
            session: None,
            volume: Arc::default(),
            open_audio: midnight_media_output::open_default,
            audio: false,
            duration: None,
            wants_play: false,
            controls: ControlProgress::default(),
            presentation: opening(session),
            notice: None,
            retirement_error: None,
        }
    }

    /// A view whose decoder broker is the test: it answers the returned requests
    /// through the returned replies, and `open_audio` stands in for the device.
    #[cfg(test)]
    fn brokered_for_test(
        session: MediaSessionId,
        volume: Arc<AudioVolume>,
        open_audio: OpenAudio,
    ) -> (Self, SyncSender<Reply>, Receiver<DecoderRequest>) {
        let (command, requests) = mpsc::sync_channel(1);
        let (results, replies) = mpsc::sync_channel(2);
        let view = Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            command: Some(command),
            size: None,
            replies,
            broker: None,
            decoder_finished: Arc::new(AtomicBool::new(false)),
            session: None,
            volume,
            open_audio,
            audio: false,
            duration: None,
            wants_play: false,
            controls: ControlProgress::default(),
            presentation: opening(session),
            notice: None,
            retirement_error: None,
        };
        (view, results, requests)
    }

    /// Start `first` of the installed helpers on `file`. The helper starts at once, while
    /// the viewer still lays out its canvas; it opens the video when [`Self::show`] gives
    /// it the canvas size.
    pub(crate) fn start(
        installation: VideoInstallation,
        file: CapturedFile,
        generation: u64,
        first: VideoWorker,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        volume: Arc<AudioVolume>,
    ) -> Result<Self, String> {
        let id = NonZeroU64::new(generation).ok_or("Invalid video session identity")?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let decoder_finished = Arc::new(AtomicBool::new(false));
        let finished = decoder_finished.clone();
        let (command, incoming) = mpsc::sync_channel(1);
        let (results, replies) = mpsc::sync_channel(2);
        let (size, sized) = mpsc::sync_channel(1);
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
                let result = run(
                    Opening {
                        installation,
                        file,
                        first,
                        session: SessionGeneration(id),
                        size: sized,
                    },
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
            size: Some(size),
            replies,
            broker: Some(broker),
            decoder_finished,
            session: None,
            volume,
            open_audio: midnight_media_output::open_default,
            audio: false,
            duration: None,
            wants_play: false,
            controls: ControlProgress::default(),
            presentation: opening(MediaSessionId(generation)),
            notice: None,
            retirement_error: None,
        })
    }

    /// The canvas is laid out at `width` x `height` physical pixels: the helper opens the
    /// video and decodes frames within these bounds. Only the first call counts.
    pub(crate) fn show(&mut self, width: u32, height: u32) {
        if let Some(size) = self.size.take() {
            // The broker takes the one size the channel holds; a broker that already
            // stopped reports its own failure.
            let _ = size.try_send(OutputBounds {
                width: width.clamp(1, 1920),
                height: height.clamp(1, 1080),
            });
        }
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
                        VideoSession::new(identity, metadata, self.volume.clone(), self.open_audio);
                    self.notice = session.audio_notice().map(str::to_owned);
                    self.audio = session.plays_audio();
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
        snapshot.audio = self.audio;
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
        // A broker still waiting for the canvas size stops waiting.
        self.size.take();
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

/// What the broker opens: `file` in the `first` helper, then the other when the first
/// cannot open it, at the canvas size size delivers.
struct Opening {
    installation: VideoInstallation,
    file: CapturedFile,
    first: VideoWorker,
    session: SessionGeneration,
    size: Receiver<OutputBounds>,
}

fn run(
    opening: Opening,
    incoming: Receiver<DecoderRequest>,
    cancel: &Arc<AtomicBool>,
    publish: &impl Fn(Reply) -> io::Result<()>,
) -> BrokerResult {
    let Opening {
        installation,
        file,
        first,
        session,
        size,
    } = opening;
    let mut retirement = None;
    let playback = (|| {
        // The owners and the transports share one consuming capture wrapper.
        let source = CapturedVideoSource::new(file, false);
        let launch = |which, retirement: &mut Option<VideoRetirement>| {
            let deadline = Instant::now() + STARTUP;
            let permit = DecoderPermit::acquire(cancel, deadline)?;
            // The platform helper also maps the system's media frameworks.
            let memory_bytes = match which {
                VideoWorker::Bundled => 512 * 1024 * 1024,
                VideoWorker::Platform => 1024 * 1024 * 1024,
            };
            let process = VideoProcess::launch(
                &installation,
                which,
                ResourcePolicy {
                    memory_bytes,
                    cpu: CpuBudget::NoAdditionalLimit,
                },
                cancel.clone(),
                deadline,
                (source.clone(), permit),
            )?;
            *retirement = Some(process.retirement());
            Ok::<_, io::Error>(process)
        };
        // The first helper starts while the viewer lays out its canvas; only opening the
        // video needs the canvas size. A closed viewer drops the sender.
        let process = launch(first, &mut retirement);
        let bounds = size
            .recv()
            .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))?;
        let open = |process: io::Result<VideoProcess>| {
            VideoTransport::open(
                process?,
                source.clone(),
                session,
                bounds,
                VideoIo {
                    deadline: Instant::now() + STARTUP,
                    cancelled: cancel,
                },
            )
        };
        let mut transport = match open(process) {
            Ok(transport) => transport,
            Err(error) => {
                let platform = installation.platform_worker().is_some();
                let Some(second) = fallback_worker(first, &error, platform) else {
                    return Err(error);
                };
                // The first helper retires and returns its decoder slot before the second.
                if let Some(finished) = retirement.take() {
                    finished.wait(Instant::now() + STARTUP)?;
                }
                open(launch(second, &mut retirement))
                    .map_err(|later| both_failed(first, error, later))?
            }
        };
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
    let retired = retirement
        .as_ref()
        .map_or(Ok(()), |finished| finished.wait(Instant::now() + RETIRE));
    // A helper that stopped during playback: its confirmed exit status says why.
    let playback = match &retirement {
        Some(finished) => playback.map_err(|error| finished.explain(error)),
        None => playback,
    };
    BrokerResult { playback, retired }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_media_host::{
        AudioPause, DeviceClock, Playback,
        video_transport::{AudioTrack, PcmBlock, PullResult, SeekEpoch},
    };
    use std::cell::RefCell;

    thread_local! {
        /// The samples the test device accepted on this test's thread.
        static WRITTEN: RefCell<Vec<i16>> = const { RefCell::new(Vec::new()) };
    }

    /// Accepts and keeps every frame queued, recording its samples.
    struct Recording;

    impl AudioSink for Recording {
        fn start(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn pause(&mut self) -> io::Result<AudioPause> {
            Ok(AudioPause::Retained(self.sample_clock()?))
        }
        fn reset(&mut self) -> io::Result<()> {
            WRITTEN.with_borrow_mut(Vec::clear);
            Ok(())
        }
        fn write(&mut self, interleaved: &[i16]) -> io::Result<usize> {
            WRITTEN.with_borrow_mut(|written| written.extend_from_slice(interleaved));
            Ok(interleaved.len() / 2)
        }
        fn sample_clock(&mut self) -> io::Result<DeviceClock> {
            Ok(DeviceClock {
                played_frames: 0,
                queued_frames: WRITTEN.with_borrow(Vec::len) / 2,
            })
        }
    }

    fn recording() -> io::Result<Box<dyn AudioSink>> {
        Ok(Box::new(Recording))
    }

    fn pulled(identity: VideoIdentity, first_frame: u64, frames: Vec<VideoFrame>) -> Reply {
        Reply::Finished(DecoderReply::Pulled(PullResult {
            identity,
            frames,
            pcm: Some(PcmBlock {
                first_frame,
                samples: vec![8000; 8].into_boxed_slice(),
            }),
            end_of_media: false,
        }))
    }

    #[test]
    fn the_window_volume_reaches_the_audio_written_for_its_video() {
        let volume = Arc::new(AudioVolume::default());
        let (mut view, replies, requests) =
            VideoView::brokered_for_test(MediaSessionId(1), volume.clone(), recording);
        let identity = VideoIdentity {
            session: SessionGeneration(NonZeroU64::new(1).unwrap()),
            epoch: SeekEpoch(NonZeroU64::new(1).unwrap()),
        };
        let metadata = VideoMetadata {
            duration: Some(Duration::from_secs(4)),
            width: 2,
            height: 1,
            audio: AudioTrack::Qualified,
        };
        replies.send(Reply::Opened(identity, metadata)).unwrap();
        let opened = view.poll().unwrap().unwrap();
        assert!(
            opened.snapshot.audio,
            "the controls learn that it has sound"
        );
        assert!(matches!(requests.try_recv(), Ok(DecoderRequest::Pull(_))));
        let frame = VideoFrame {
            presentation: Duration::ZERO,
            width: 2,
            height: 1,
            rgba: vec![0; 8].into_boxed_slice(),
        };
        replies.send(pulled(identity, 0, vec![frame])).unwrap();

        // The window's level, set while the first block waits, scales it.
        volume.set(50, false).unwrap();
        let play = MediaControl {
            session: MediaSessionId(1),
            id: MediaControlId(1),
            action: MediaAction::SetPlaying(true),
        };
        assert!(view.apply_control(play).unwrap());
        view.poll().unwrap();
        assert_eq!(WRITTEN.with_borrow(Vec::clone), [1000; 8]);

        // Muting during playback silences the next block at once.
        assert!(matches!(requests.try_recv(), Ok(DecoderRequest::Pull(_))));
        replies.send(pulled(identity, 4, Vec::new())).unwrap();
        volume.set(50, true).unwrap();
        view.poll().unwrap();
        assert_eq!(WRITTEN.with_borrow(|written| written[8..].to_vec()), [0; 8]);
    }

    #[test]
    fn a_video_without_sound_reports_none() {
        let (mut view, replies, _requests) =
            VideoView::brokered_for_test(MediaSessionId(2), Arc::default(), || {
                panic!("a video without an audio track opens no device")
            });
        let identity = VideoIdentity {
            session: SessionGeneration(NonZeroU64::new(2).unwrap()),
            epoch: SeekEpoch(NonZeroU64::new(1).unwrap()),
        };
        let metadata = VideoMetadata {
            duration: Some(Duration::from_secs(4)),
            width: 2,
            height: 1,
            audio: AudioTrack::Absent,
        };
        replies.send(Reply::Opened(identity, metadata)).unwrap();
        view.poll().unwrap();
        assert!(!view.presentation().audio);
    }

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
