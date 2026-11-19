use super::*;
use crate::playback::Frame;
use crate::playback_session::{Command, Completion, Event};
use std::io::Cursor;

fn tag(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn header(kind: u8, length: u32) -> Vec<u8> {
    let mut bytes = vec![b'M', b'M', b'V', b'2', kind, 0, 0, 0];
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes
}

fn packet(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = header(kind, payload.len() as u32);
    bytes.extend_from_slice(payload);
    bytes
}

fn numbers(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn opening(generation: u64, width: u32, height: u32) -> Vec<u8> {
    let mut body = numbers(&[0x0102_0304_0506_0708, generation]);
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&height.to_le_bytes());
    packet(1, &body)
}

fn reply(request: u64, result: u8, data: &[u8]) -> Vec<u8> {
    let mut body = numbers(&[request]);
    body.push(result);
    body.extend_from_slice(data);
    packet(5, &body)
}

fn invalid(bytes: Vec<u8>) -> io::ErrorKind {
    read_message(&mut Cursor::new(bytes))
        .err()
        .expect("invalid wire input")
        .kind()
}

#[test]
fn wire_inbound_commands_preserve_identity_tags_and_little_endian_fields() {
    let mut input = opening(9, 1280, 720);
    assert_eq!(&input[..12], b"MMV2\x01\0\0\0\x18\0\0\0");
    let mut pull = numbers(&[11, 9]);
    pull.push(4);
    pull.extend_from_slice(&0u16.to_le_bytes());
    input.extend(packet(2, &pull));
    input.extend(packet(3, &numbers(&[12, 10, 5_000_123])));
    input.extend(packet(4, &numbers(&[13])));
    let mut input = Cursor::new(input);
    let Incoming::Open {
        source_length,
        generation,
        width,
        height,
    } = read_message(&mut input).unwrap().unwrap()
    else {
        panic!("Open")
    };
    assert_eq!(
        (source_length, generation.get(), width, height),
        (0x0102_0304_0506_0708, 9, 1280, 720)
    );
    let Incoming::Session(request) = read_message(&mut input).unwrap().unwrap() else {
        panic!("Pull")
    };
    assert_eq!(request.id.get(), 11);
    let Command::Pull {
        generation,
        frames,
        pcm_frames: 0,
    } = request.command
    else {
        panic!("Pull command")
    };
    assert_eq!((generation.get(), frames), (9, 4));
    let Incoming::Session(request) = read_message(&mut input).unwrap().unwrap() else {
        panic!("Seek")
    };
    assert_eq!(request.id.get(), 12);
    let Command::Seek {
        generation,
        position_micros,
    } = request.command
    else {
        panic!("Seek command")
    };
    assert_eq!((generation.get(), position_micros), (10, 5_000_123));
    let Incoming::Session(request) = read_message(&mut input).unwrap().unwrap() else {
        panic!("Close")
    };
    assert_eq!(request.id.get(), 13);
    assert!(matches!(request.command, Command::Close));
    assert!(read_message(&mut input).unwrap().is_none());
}

#[test]
fn wire_read_replies_distinguish_empty_success_clean_eof_and_failures() {
    for body in [b"owned raw bytes".as_slice(), &[]] {
        let Incoming::ReadReply {
            read_request,
            result,
        } = read_message(&mut Cursor::new(reply(7, 0, body)))
            .unwrap()
            .unwrap()
        else {
            panic!("ReadReply")
        };
        assert_eq!(read_request.get(), 7);
        assert_eq!(result.unwrap(), body);
    }
    for (code, expected) in [
        (1, io::ErrorKind::PermissionDenied),
        (2, io::ErrorKind::Interrupted),
        (3, io::ErrorKind::Other),
    ] {
        let Incoming::ReadReply {
            read_request,
            result,
        } = read_message(&mut Cursor::new(reply(8, code, &[])))
            .unwrap()
            .unwrap()
        else {
            panic!("ReadReply failure")
        };
        assert_eq!(read_request.get(), 8);
        assert_eq!(result.unwrap_err().kind(), expected);
    }
    assert!(read_message(&mut Cursor::new([])).unwrap().is_none());
}

struct HeaderOnly(Cursor<Vec<u8>>);
impl Read for HeaderOnly {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        assert!(
            self.0.position() < 12,
            "invalid header attempted to read a payload"
        );
        self.0.read(bytes)
    }
}

#[test]
fn wire_malformed_or_over_budget_header_is_rejected_before_payload_read() {
    let mut bad_magic = header(1, 24);
    bad_magic[0] = b'X';
    let mut reserved = header(1, 24);
    reserved[6] = 1;
    for bytes in [
        bad_magic,
        reserved,
        header(0, 0),
        header(0x81, 24),
        header(1, 25),
        header(2, 18),
        header(3, 23),
        header(4, 9),
        header(5, 8),
        header(5, 9 + 256 * 1024 + 1),
        header(5, u32::MAX),
    ] {
        let error = read_message(&mut HeaderOnly(Cursor::new(bytes)))
            .err()
            .expect("reject header");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn wire_rejects_zero_tags_invalid_credit_geometry_and_error_bodies() {
    for (generation, width, height) in [(0, 2, 1), (1, 0, 1), (1, 4097, 1), (1, 4096, 1025)] {
        assert_eq!(
            invalid(opening(generation, width, height)),
            io::ErrorKind::InvalidData
        );
    }
    for (request, generation, frames) in [(0, 1, 1), (1, 0, 1), (1, 1, 0), (1, 1, 5)] {
        let mut body = numbers(&[request, generation]);
        body.push(frames);
        body.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(invalid(packet(2, &body)), io::ErrorKind::InvalidData);
    }
    for body in [numbers(&[0, 1, 12]), numbers(&[1, 0, 12])] {
        assert_eq!(invalid(packet(3, &body)), io::ErrorKind::InvalidData);
    }
    assert_eq!(
        invalid(packet(4, &numbers(&[0]))),
        io::ErrorKind::InvalidData
    );
    for bytes in [
        reply(0, 0, &[]),
        reply(1, 4, &[]),
        reply(1, 1, b"x"),
        reply(1, 2, b"x"),
        reply(1, 3, b"x"),
    ] {
        assert_eq!(invalid(bytes), io::ErrorKind::InvalidData);
    }
}

#[test]
fn wire_truncated_header_or_payload_never_reports_clean_eof() {
    for complete in [opening(1, 2, 1), reply(1, 0, b"owned payload")] {
        for end in 1..complete.len() {
            let error = read_message(&mut Cursor::new(&complete[..end]))
                .err()
                .expect("truncated frame");
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "prefix {end}");
        }
    }
}

#[test]
fn wire_outgoing_metadata_status_and_unknown_duration_match_exact_bytes() {
    let mut actual = Vec::new();
    write_message(
        &mut actual,
        Outgoing::Opened {
            generation: tag(3),
            duration_micros: None,
            width: 2,
            height: 1,
            audio: AudioTrack::Absent,
        },
    )
    .unwrap();
    let mut body = numbers(&[3, u64::MAX]);
    body.extend_from_slice(&2_u32.to_le_bytes());
    body.extend_from_slice(&1_u32.to_le_bytes());
    body.push(0);
    assert_eq!(actual, packet(0x81, &body));
    actual.clear();
    write_message(
        &mut actual,
        Outgoing::Opened {
            generation: tag(3),
            duration_micros: Some(40_000),
            width: 2,
            height: 1,
            audio: AudioTrack::Absent,
        },
    )
    .unwrap();
    body[8..16].copy_from_slice(&40_000_u64.to_le_bytes());
    assert_eq!(actual, packet(0x81, &body));
    actual.clear();
    write_message(
        &mut actual,
        Outgoing::Read {
            read_request: tag(7),
            offset: 0x10_0000_0008,
            length: 256 * 1024,
        },
    )
    .unwrap();
    let mut body = numbers(&[7, 0x10_0000_0008]);
    body.extend_from_slice(&(256_u32 * 1024).to_le_bytes());
    assert_eq!(actual, packet(0x82, &body));
    for (completion, status) in [
        (Completion::Served, 0),
        (Completion::EndOfMedia, 1),
        (Completion::Superseded, 2),
    ] {
        actual.clear();
        let event = Event::Completed {
            request: tag(8),
            generation: tag(3),
            completion,
        };
        write_message(&mut actual, Outgoing::Event(&event)).unwrap();
        let mut body = numbers(&[8, 3]);
        body.push(status);
        assert_eq!(actual, packet(0x84, &body));
    }
    actual.clear();
    write_message(
        &mut actual,
        Outgoing::Failed {
            request: 0,
            generation: 0,
            cause: 255,
        },
    )
    .unwrap();
    let mut body = numbers(&[0, 0]);
    body.push(255);
    assert_eq!(actual, packet(0x85, &body));
}

#[test]
fn wire_opened_natural_dimensions_have_a_separate_budget_from_output_frames() {
    let mut actual = Vec::new();
    write_message(
        &mut actual,
        Outgoing::Opened {
            generation: tag(3),
            duration_micros: Some(40_000),
            width: 8192,
            height: 4096,
            audio: AudioTrack::Absent,
        },
    )
    .unwrap();
    let mut body = numbers(&[3, 40_000]);
    body.extend_from_slice(&8192_u32.to_le_bytes());
    body.extend_from_slice(&4096_u32.to_le_bytes());
    body.push(0);
    assert_eq!(actual, packet(0x81, &body));
    actual.clear();
    assert_eq!(
        write_message(
            &mut actual,
            Outgoing::Opened {
                generation: tag(3),
                duration_micros: None,
                width: 8192,
                height: 4097,
                audio: AudioTrack::Absent
            }
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(actual.is_empty());
    assert_eq!(invalid(opening(3, 8192, 4096)), io::ErrorKind::InvalidData);
    let event = Event::Video {
        request: tag(1),
        generation: tag(3),
        frame: Frame {
            presentation_micros: 0,
            width: 8192,
            height: 4096,
            rgba: Vec::new(),
        },
    };
    assert_eq!(
        write_message(&mut actual, Outgoing::Event(&event))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(actual.is_empty());
}

struct Capture {
    original: *const u8,
    raw_length: usize,
    borrowed_raw: bool,
    flushes: usize,
    bytes: Vec<u8>,
}
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.borrowed_raw |= bytes.as_ptr() == self.original && bytes.len() == self.raw_length;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

fn frame() -> Frame {
    Frame {
        presentation_micros: 123_456,
        width: 2,
        height: 1,
        rgba: vec![1, 2, 3, 0, 200, 150, 100, 17],
    }
}

#[test]
fn wire_video_exports_the_original_raw_rgba_borrow_without_channel_or_alpha_changes() {
    let frame = frame();
    let mut output = Capture {
        original: frame.rgba.as_ptr(),
        raw_length: frame.rgba.len(),
        borrowed_raw: false,
        flushes: 0,
        bytes: Vec::new(),
    };
    let event = Event::Video {
        request: tag(5),
        generation: tag(9),
        frame,
    };
    write_message(&mut output, Outgoing::Event(&event)).unwrap();
    assert!(
        output.borrowed_raw,
        "video must borrow original pixels rather than clone a frame"
    );
    assert_eq!(output.flushes, 1);
    let mut body = numbers(&[5, 9]);
    body.extend_from_slice(&123_456_i64.to_le_bytes());
    body.extend_from_slice(&2_u32.to_le_bytes());
    body.extend_from_slice(&1_u32.to_le_bytes());
    body.extend_from_slice(&[1, 2, 3, 0, 200, 150, 100, 17]);
    assert_eq!(output.bytes, packet(0x83, &body));
}

#[test]
fn wire_invalid_outgoing_metadata_or_frame_is_rejected_before_first_write() {
    for invalid in [
        Frame {
            presentation_micros: -1,
            ..frame()
        },
        Frame {
            width: 0,
            ..frame()
        },
        Frame {
            width: 4097,
            ..frame()
        },
        Frame {
            width: 4096,
            height: 1025,
            ..frame()
        },
        Frame {
            rgba: vec![0; 7],
            ..frame()
        },
        Frame {
            rgba: vec![0; 9],
            ..frame()
        },
    ] {
        let event = Event::Video {
            request: tag(1),
            generation: tag(1),
            frame: invalid,
        };
        let mut output = Vec::new();
        assert_eq!(
            write_message(&mut output, Outgoing::Event(&event))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(output.is_empty());
    }
    for invalid in [
        Outgoing::Opened {
            generation: tag(1),
            duration_micros: Some(u64::MAX),
            width: 2,
            height: 1,
            audio: AudioTrack::Absent,
        },
        Outgoing::Opened {
            generation: tag(1),
            duration_micros: None,
            width: 0,
            height: 1,
            audio: AudioTrack::Absent,
        },
        Outgoing::Read {
            read_request: tag(1),
            offset: 0,
            length: 256 * 1024 + 1,
        },
        Outgoing::Read {
            read_request: tag(1),
            offset: u64::MAX,
            length: 1,
        },
    ] {
        let mut output = Vec::new();
        assert_eq!(
            write_message(&mut output, invalid).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(output.is_empty());
    }
}

struct Short<R> {
    inner: R,
    interrupted: bool,
}
impl<R: Read> Read for Short<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = bytes.len().min(31);
        self.inner.read(&mut bytes[..count])
    }
}
impl<W: Write> Write for Short<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.inner.write(&bytes[..bytes.len().min(3)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[test]
fn wire_maximum_read_body_and_short_interrupted_io_preserve_framing() {
    let raw = vec![0xa7; 256 * 1024];
    let mut bytes = reply(4, 0, &raw);
    bytes.extend(packet(4, &numbers(&[5])));
    let mut input = Short {
        inner: Cursor::new(bytes),
        interrupted: false,
    };
    let Incoming::ReadReply {
        read_request,
        result,
    } = read_message(&mut input).unwrap().unwrap()
    else {
        panic!("maximum ReadReply")
    };
    assert_eq!(read_request.get(), 4);
    assert!(result.unwrap() == raw, "maximum raw reply bytes changed");
    let Incoming::Session(request) = read_message(&mut input).unwrap().unwrap() else {
        panic!("following Close")
    };
    assert_eq!(request.id.get(), 5);
    assert!(matches!(request.command, Command::Close));
    let event = Event::Video {
        request: tag(1),
        generation: tag(2),
        frame: frame(),
    };
    let mut exact = Vec::new();
    write_message(&mut exact, Outgoing::Event(&event)).unwrap();
    let mut short = Short {
        inner: Vec::new(),
        interrupted: false,
    };
    write_message(&mut short, Outgoing::Event(&event)).unwrap();
    assert_eq!(short.inner, exact);
}

#[test]
fn pcm_and_video_credits_are_independent_and_bounded() {
    let mut demand = numbers(&[3, 1]);
    demand.push(0);
    demand.extend_from_slice(&120u16.to_le_bytes());
    let Some(Incoming::Session(Request {
        command: Command::Pull {
            frames, pcm_frames, ..
        },
        ..
    })) = read_message(&mut Cursor::new(
        header(2, 19).into_iter().chain(demand).collect::<Vec<_>>(),
    ))
    .unwrap()
    else {
        panic!("expected bounded PCM-only demand");
    };
    assert_eq!((frames, pcm_frames), (0, 120));
    let block = crate::playback_audio::Block {
        first_frame: 80,
        samples: vec![1, -2],
    };
    let event = Event::Pcm {
        request: tag(3),
        generation: tag(1),
        block,
    };
    let mut sent = Vec::new();
    write_message(&mut sent, Outgoing::Event(&event)).unwrap();
    let mut expected = header(0x86, 30);
    expected.extend_from_slice(&numbers(&[3, 1, 80]));
    expected.extend_from_slice(&[1, 0, 1, 0, 254, 255]);
    assert_eq!(sent, expected);
}
