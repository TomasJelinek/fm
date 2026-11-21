use super::*;
use std::{
    cell::RefCell,
    io::{Cursor, Read},
    rc::Rc,
};

#[derive(Default)]
struct Trace {
    sent: Vec<u8>,
    reads: Vec<(u64, usize)>,
    retired: usize,
}

struct Pipe {
    incoming: Cursor<Vec<u8>>,
    trace: Rc<RefCell<Trace>>,
}

impl RestrictedVideoPipe for Pipe {
    fn read_exact(&mut self, bytes: &mut [u8], _: VideoIo<'_>) -> io::Result<()> {
        self.incoming.read_exact(bytes)
    }

    fn send(&mut self, parts: &[&[u8]], _: VideoIo<'_>) -> io::Result<()> {
        let mut trace = self.trace.borrow_mut();
        for part in parts {
            trace.sent.extend_from_slice(part);
        }
        Ok(())
    }

    fn retire(&mut self) {
        self.trace.borrow_mut().retired += 1;
    }
}

struct Source {
    bytes: &'static [u8],
    deny: bool,
    trace: Rc<RefCell<Trace>>,
}

impl VideoSource for Source {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, bytes: &mut [u8], _: VideoIo<'_>) -> io::Result<usize> {
        self.trace.borrow_mut().reads.push((offset, bytes.len()));
        if self.deny {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let offset = usize::try_from(offset).unwrap();
        bytes.copy_from_slice(&self.bytes[offset..offset + bytes.len()]);
        Ok(bytes.len())
    }
}

fn stream(incoming: Vec<u8>, deny: bool) -> (Pipe, Source, Rc<RefCell<Trace>>) {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let pipe = Pipe {
        incoming: Cursor::new(incoming),
        trace: trace.clone(),
    };
    let source = Source {
        bytes: &[1, 2, 4, 5, 6],
        deny,
        trace: trace.clone(),
    };
    (pipe, source, trace)
}

fn control(cancelled: &AtomicBool) -> VideoIo<'_> {
    VideoIo {
        deadline: Instant::now() + Duration::from_secs(2),
        cancelled,
    }
}

fn open(pipe: Pipe, source: Source, io: VideoIo<'_>) -> io::Result<VideoTransport<Pipe, Source>> {
    VideoTransport::open(
        pipe,
        source,
        SessionGeneration(NonZeroU64::new(9).unwrap()),
        OutputBounds {
            width: 2,
            height: 2,
        },
        io,
    )
}

fn append_packet(stream: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    stream.extend_from_slice(b"MMV2");
    stream.extend_from_slice(&[kind, 0, 0, 0]);
    stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    stream.extend_from_slice(payload);
}

fn opened(track: u8) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&u64::MAX.to_le_bytes());
    payload.extend_from_slice(&1920u32.to_le_bytes());
    payload.extend_from_slice(&1080u32.to_le_bytes());
    payload.push(track);
    let mut stream = Vec::new();
    append_packet(&mut stream, 0x81, &payload);
    stream
}

fn tagged(request: u64, epoch: u64) -> Vec<u8> {
    let mut payload = request.to_le_bytes().to_vec();
    payload.extend_from_slice(&epoch.to_le_bytes());
    payload
}

fn read_request(stream: &mut Vec<u8>, request: u64, offset: u64, length: u32) {
    let mut payload = tagged(request, offset);
    payload.extend_from_slice(&length.to_le_bytes());
    append_packet(stream, 0x82, &payload);
}

fn terminal(stream: &mut Vec<u8>, kind: u8, request: u64, epoch: u64, status: u8) {
    let mut packet = tagged(request, epoch);
    packet.push(status);
    append_packet(stream, kind, &packet);
}

fn failed(stream: &mut Vec<u8>, request: u64, epoch: u64, cause: [u8; 3]) {
    let mut packet = tagged(request, epoch);
    packet.extend_from_slice(&cause);
    append_packet(stream, 0x85, &packet);
}

fn video(stream: &mut Vec<u8>, request: u64, epoch: u64, width: u32, height: u32) {
    let mut payload = tagged(request, epoch);
    payload.extend_from_slice(&27i64.to_le_bytes());
    payload.extend_from_slice(&width.to_le_bytes());
    payload.extend_from_slice(&height.to_le_bytes());
    payload.extend_from_slice(&[1, 2, 3, 255]);
    append_packet(stream, 0x83, &payload);
}

fn pcm(stream: &mut Vec<u8>, request: u64, epoch: u64, first_frame: u64) {
    let mut payload = tagged(request, epoch);
    payload.extend_from_slice(&first_frame.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&[0x00, 0x80, 0xff, 0x7f]);
    append_packet(stream, 0x86, &payload);
}

fn demand(video_frames: u8, audio_frames: u16) -> PullCredit {
    PullCredit {
        video_frames,
        audio: NonZeroU16::new(audio_frames).map_or(AudioDemand::Discard, AudioDemand::Frames),
    }
}

#[test]
fn open_and_read_reply_match_the_worker_capture_fixture() {
    let mut incoming = Vec::new();
    // Range and bytes from the child's broker fixture with the MMV2 magic.
    read_request(&mut incoming, 1, 2, 3);
    incoming.extend_from_slice(&opened(0));
    let (pipe, source, trace) = stream(incoming, false);
    let stop = AtomicBool::new(false);
    let broker = open(pipe, source, control(&stop)).unwrap();
    let info = broker.metadata();
    assert_eq!(info.audio, AudioTrack::Absent);
    assert_eq!(info.duration, None);
    assert_eq!((info.width, info.height), (1920, 1080));
    assert_eq!(broker.identity().session.0.get(), 9);
    assert_eq!(broker.identity().epoch.get(), 1);
    let observed = trace.borrow();
    assert_eq!(observed.reads, [(2, 3)]);
    let mut expected = b"MMV2\x01\0\0\0\x18\0\0\0".to_vec();
    expected.extend_from_slice(&5u64.to_le_bytes());
    expected.extend_from_slice(&1u64.to_le_bytes());
    expected.extend_from_slice(&[2, 0, 0, 0, 2, 0, 0, 0]);
    expected.extend_from_slice(b"MMV2\x05\0\0\0\x0c\0\0\0\x01\0\0\0\0\0\0\0\0\x04\x05\x06");
    assert_eq!(observed.sent, expected);
}

#[test]
fn pull_and_seek_keep_session_and_epoch_distinct_and_restart_pcm() {
    let mut incoming = opened(1);
    video(&mut incoming, 1, 1, 1, 1);
    pcm(&mut incoming, 1, 1, 0);
    terminal(&mut incoming, 0x84, 1, 1, 0);
    terminal(&mut incoming, 0x84, 2, 2, 0);
    pcm(&mut incoming, 3, 2, 0);
    terminal(&mut incoming, 0x84, 3, 2, 1);
    let (pipe, source, trace) = stream(incoming, false);
    let stop = AtomicBool::new(false);
    let io = control(&stop);
    let mut broker = open(pipe, source, io).unwrap();

    let pulled = broker.pull(demand(1, 2), io).unwrap();
    assert_eq!(pulled.identity, broker.identity());
    assert!(!pulled.end_of_media);
    assert_eq!(pulled.frames[0].presentation, Duration::from_micros(27));
    assert_eq!(pulled.frames[0].rgba.as_ref(), [1, 2, 3, 255]);
    let block = pulled.pcm.unwrap();
    assert_eq!(block.first_frame, 0);
    assert_eq!(block.samples.as_ref(), [i16::MIN, i16::MAX]);

    let epoch = broker.seek(Duration::from_micros(900), io).unwrap();
    assert_eq!(epoch.get(), 2);
    let next = broker.pull(demand(0, 1), io).unwrap();
    assert_eq!(next.identity.session, pulled.identity.session);
    assert_eq!(next.identity.epoch, epoch);
    assert_eq!(next.pcm.unwrap().first_frame, 0);
    assert!(next.end_of_media);
    assert!(next.frames.is_empty());

    let mut expected = Vec::new();
    let mut pull = tagged(1, 1);
    pull.extend_from_slice(&[1, 2, 0]);
    append_packet(&mut expected, 2, &pull);
    let mut seek = tagged(2, 2);
    seek.extend_from_slice(&900u64.to_le_bytes());
    append_packet(&mut expected, 3, &seek);
    let mut audio_only = tagged(3, 2);
    audio_only.extend_from_slice(&[0, 1, 0]);
    append_packet(&mut expected, 2, &audio_only);
    assert_eq!(&trace.borrow().sent[36..], expected.as_slice());
}

#[test]
fn out_of_range_or_replayed_read_retires_without_touching_source() {
    for (id, offset, length) in [(1, 4, 2), (1, 5, 1), (1, 0, 0), (2, 0, 1)] {
        let mut incoming = Vec::new();
        read_request(&mut incoming, id, offset, length);
        let (pipe, source, trace) = stream(incoming, false);
        let stop = AtomicBool::new(false);
        let failure = open(pipe, source, control(&stop)).err().unwrap();
        assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
        assert!(trace.borrow().reads.is_empty());
        assert_eq!(trace.borrow().retired, 1);
        assert_eq!(trace.borrow().sent.len(), 36);
    }
}

#[test]
fn read_denial_notifies_exact_read_and_serves_no_more_data() {
    let mut incoming = Vec::new();
    read_request(&mut incoming, 1, 2, 3);
    failed(&mut incoming, 0, 1, [1, 0, 0]);
    let (pipe, source, trace) = stream(incoming, true);
    let stop = AtomicBool::new(false);
    let failure = open(pipe, source, control(&stop)).err().unwrap();
    assert_eq!(failure.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        &trace.borrow().sent[36..],
        b"MMV2\x05\0\0\0\x09\0\0\0\x01\0\0\0\0\0\0\0\x01"
    );
    assert_eq!(trace.borrow().retired, 1);
}

#[test]
fn excess_pcm_or_invalid_video_is_rejected_before_exposing_the_pull() {
    for fault in 0..5 {
        let mut incoming = opened(1);
        match fault {
            0 => {
                pcm(&mut incoming, 1, 1, 0);
                pcm(&mut incoming, 1, 1, 1);
            }
            1 => pcm(&mut incoming, 1, 1, 4),
            2 => video(&mut incoming, 1, 1, 3, 1),
            3 => video(&mut incoming, 1, 2, 1, 1),
            _ => video(&mut incoming, 1, 1, u32::MAX, u32::MAX),
        }
        terminal(&mut incoming, 0x84, 1, 1, 0);
        let (pipe, source, trace) = stream(incoming, false);
        let stop = AtomicBool::new(false);
        let io = control(&stop);
        let mut broker = open(pipe, source, io).unwrap();
        assert_eq!(
            broker.pull(demand(1, 2), io).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(trace.borrow().retired, 1);
        assert_eq!(
            broker.pull(demand(1, 2), io).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}

#[test]
fn unqualified_audio_requires_explicit_video_only_credit() {
    let mut incoming = opened(2);
    terminal(&mut incoming, 0x84, 1, 1, 1);
    let (pipe, source, trace) = stream(incoming, false);
    let stop = AtomicBool::new(false);
    let io = control(&stop);
    let mut broker = open(pipe, source, io).unwrap();
    assert_eq!(broker.metadata().audio, AudioTrack::Unqualified);
    assert_eq!(
        broker.pull(demand(1, 1), io).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(trace.borrow().sent.len(), 36);
    assert!(broker.pull(demand(1, 0), io).unwrap().end_of_media);
    assert_eq!(&trace.borrow().sent[36 + 12 + 16..], &[1, 0, 0]);
}

#[test]
fn a_cancelled_or_closed_transport_retires_once() {
    for cancelled in [false, true] {
        let (pipe, source, trace) = stream(opened(0), false);
        let stop = AtomicBool::new(false);
        let io = control(&stop);
        let mut broker = open(pipe, source, io).unwrap();
        if cancelled {
            stop.store(true, Ordering::Release);
            assert_eq!(
                broker.pull(demand(1, 0), io).unwrap_err().kind(),
                io::ErrorKind::Interrupted
            );
            assert_eq!(trace.borrow().sent.len(), 36);
        } else {
            broker.close(io).unwrap();
            assert_eq!(
                &trace.borrow().sent[36..],
                b"MMV2\x04\0\0\0\x08\0\0\0\x01\0\0\0\0\0\0\0"
            );
        }
        broker.close(io).unwrap();
        drop(broker);
        assert_eq!(trace.borrow().retired, 1);
    }
}

#[test]
fn a_missing_decoder_at_open_names_its_codec_and_what_the_helper_tried() {
    let mut incoming = Vec::new();
    failed(&mut incoming, 0, 1, [5, 2, 1]);
    let (pipe, source, trace) = stream(incoming, false);
    let stop = AtomicBool::new(false);
    let failure = open(pipe, source, control(&stop)).err().unwrap();
    assert_eq!(failure.kind(), io::ErrorKind::Unsupported);
    assert_eq!(
        MissingDecoder::find(&failure),
        Some(MissingDecoder { codec: 2, tried: 1 })
    );
    assert!(failure.to_string().starts_with("This video uses HEVC; "));
    assert_eq!(trace.borrow().retired, 1);
}

#[test]
fn a_codec_on_any_other_failure_or_the_old_failure_length_is_refused() {
    for packet in [[2, 2, 1], [4, 0, 8]] {
        let mut incoming = Vec::new();
        failed(&mut incoming, 0, 1, packet);
        let (pipe, source, _) = stream(incoming, false);
        let stop = AtomicBool::new(false);
        let failure = open(pipe, source, control(&stop)).err().unwrap();
        assert_eq!(failure.kind(), io::ErrorKind::InvalidData, "{packet:?}");
        assert_eq!(MissingDecoder::find(&failure), None);
    }
    // A helper of the previous profile sends the 17-byte failure.
    let mut incoming = Vec::new();
    terminal(&mut incoming, 0x85, 0, 1, 2);
    let (pipe, source, _) = stream(incoming, false);
    let stop = AtomicBool::new(false);
    let failure = open(pipe, source, control(&stop)).err().unwrap();
    assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
}
