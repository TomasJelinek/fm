//! The Linux platform helpers as the host runs them: spawned with a cleared environment, given
//! the worker SDK's startup policy, then fed the capture through brokered reads only. The
//! fixtures are encoded at test time with the system's `ffmpeg` command.
#![cfg(all(
    target_os = "linux",
    feature = "system",
    feature = "playback-worker",
    not(feature = "bundled")
))]

use midnight_thumbnail_sdk::api::{self, Message, Request};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

const THUMBNAILER: &str = env!("CARGO_BIN_EXE_midnight-thumbnail-video-system");
const PLAYER: &str = env!("CARGO_BIN_EXE_midnight-video-playback-system");

fn ffmpeg() -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|folder| Path::new(folder).join("ffmpeg"))
        .find(|path| path.is_file())
}

/// The codecs of a fixture.
#[derive(Clone, Copy, PartialEq)]
enum Codecs {
    /// H.264 and AAC in MP4.
    H264,
    /// Windows Media Video 8 and Windows Media Audio 2 in ASF, as `.wmv` files hold them.
    WindowsMedia,
    /// MPEG-4 Part 2 in AVI with its tag replaced by an unknown one.
    UnknownTag,
}

/// Four one-second colours (red, lime, blue, white) at 25 fps with a sine track, in `codecs`.
fn fixture(name: &str, codecs: Codecs) -> Option<Vec<u8>> {
    let ffmpeg = ffmpeg()?;
    let folder =
        std::env::temp_dir().join(format!("midnight-system-helpers-{}", std::process::id()));
    std::fs::create_dir_all(&folder).ok()?;
    let output = folder.join(name);
    let colours = "color=c=red:s=320x240:r=25:d=1[a];color=c=lime:s=320x240:r=25:d=1[b];\
        color=c=blue:s=320x240:r=25:d=1[c];color=c=white:s=320x240:r=25:d=1[d];\
        [a][b][c][d]concat=n=4:v=1:a=0[v]";
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("sine=frequency=440:sample_rate=44100:duration=4")
        .args(["-filter_complex", colours, "-map", "[v]", "-map", "0:a"]);
    match codecs {
        Codecs::UnknownTag => {
            command.args(["-c:v", "mpeg4", "-c:a", "libmp3lame"]);
        }
        Codecs::WindowsMedia => {
            command.args(["-c:v", "wmv2", "-g", "25", "-c:a", "wmav2", "-f", "asf"]);
        }
        Codecs::H264 => {
            command.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "25",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
            ]);
        }
    }
    if !command.arg(&output).status().ok()?.success() {
        return None;
    }
    let mut bytes = std::fs::read(&output).ok()?;
    if codecs == Codecs::UnknownTag {
        for tag in [*b"FMP4", *b"fmp4"] {
            while let Some(at) = bytes.windows(4).position(|window| window == tag) {
                bytes[at..at + 4].copy_from_slice(b"ZQ9X");
            }
        }
    }
    Some(bytes)
}

struct Helper {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    started: Instant,
}

impl Helper {
    /// Spawn like the host's Linux launcher: no environment, root as working folder, no
    /// stderr; then the startup policy.
    fn spawn(executable: &str, policy: impl FnOnce(&mut ChildStdin) -> io::Result<()>) -> Self {
        let started = Instant::now();
        let mut child = Command::new(executable)
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("system helper starts");
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        policy(&mut input).unwrap();
        Self {
            child,
            input,
            output,
            started,
        }
    }

    /// Address space after startup, from the kernel's accounting.
    fn address_space(&self) -> String {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.child.id()))
            .unwrap_or_default();
        status
            .lines()
            .filter(|line| {
                line.starts_with("VmPeak")
                    || line.starts_with("VmSize")
                    || line.starts_with("VmRSS")
            })
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One MTH1 thumbnail through the system thumbnail helper.
fn thumbnail(bytes: &[u8], frame: u8) -> (Message, String) {
    let mut helper = Helper::spawn(THUMBNAILER, |input| {
        midnight_worker_sdk::write_limits(
            input,
            midnight_worker_sdk::Limits {
                memory_bytes: 512 * 1024 * 1024,
                cpu_seconds: 8,
            },
        )
    });
    midnight_worker_sdk::read_ready(&mut helper.output).expect("the helper entered its sandbox");
    let ready = format!(
        "ready after {:?}; {}",
        helper.started.elapsed(),
        helper.address_space()
    );
    let request = Request {
        id: 3,
        source_length: bytes.len() as u64,
        width: 64,
        height: 64,
        frame,
    };
    api::write_message(&mut helper.input, &Message::Request(request)).unwrap();
    loop {
        match api::read_message(&mut helper.output).expect("helper reply") {
            Message::Read { offset, length } => {
                let start = offset as usize;
                let end = (start + length as usize).min(bytes.len());
                api::write_message(
                    &mut helper.input,
                    &Message::Data(bytes[start..end].to_vec()),
                )
                .unwrap();
            }
            other => return (other, ready),
        }
    }
}

fn centre(width: u32, height: u32, rgba: &[u8]) -> [u8; 3] {
    let at = ((height / 2 * width + width / 2) * 4) as usize;
    [rgba[at], rgba[at + 1], rgba[at + 2]]
}

#[test]
fn the_thumbnail_helper_decodes_h264_inside_its_sandbox() {
    let Some(bytes) = fixture("h264-aac.mp4", Codecs::H264) else {
        eprintln!("skipped: no ffmpeg command to encode the fixture");
        return;
    };
    let (reply, ready) = thumbnail(&bytes, 3);
    eprintln!("thumbnail helper {ready}");
    let Message::Frame(frame) = reply else {
        panic!("expected a frame, got another reply (is a system FFmpeg or GStreamer installed?)");
    };
    assert_eq!(
        (frame.id, frame.frame, frame.width, frame.height),
        (3, 3, 64, 48)
    );
    let [r, g, b] = centre(frame.width, frame.height, &frame.rgba);
    // 55% of four seconds is in the blue second.
    assert!(b > 180 && r < 90 && g < 90, "{r} {g} {b}");
}

/// `.wmv` and `.asf` files, which the host routes to these helpers: Windows Media Video and
/// Audio in ASF through the distribution's FFmpeg (or GStreamer's ASF demuxer and gst-libav).
#[test]
fn the_helpers_decode_windows_media_inside_their_sandbox() {
    let Some(bytes) = fixture("wmv2-wmav2.wmv", Codecs::WindowsMedia) else {
        eprintln!("skipped: no ffmpeg command with Windows Media encoders");
        return;
    };
    let (reply, ready) = thumbnail(&bytes, 3);
    eprintln!("thumbnail helper {ready}");
    let Message::Frame(frame) = reply else {
        panic!("expected a frame, got another reply (is a system FFmpeg installed?)");
    };
    assert_eq!((frame.width, frame.height), (64, 48));
    let [r, g, b] = centre(frame.width, frame.height, &frame.rgba);
    assert!(b > 180 && r < 90 && g < 90, "{r} {g} {b}");
    let (mut helper, _) = open_player(&bytes);
    let (kind, opened) = next(&mut helper, &bytes);
    assert_eq!(kind, 0x81, "expected Opened, got {kind:#x} {opened:?}");
    let duration = u64_at(&opened, 8);
    assert!(
        (3_900_000..=4_200_000).contains(&duration),
        "duration {duration}"
    );
    assert_eq!((u32_at(&opened, 16), u32_at(&opened, 20)), (320, 240));
    assert_eq!(opened[24], 1, "the Windows Media Audio track is qualified");
    let (frames, samples, completion) = pull(&mut helper, &bytes, 1, 9, 4, 4800);
    assert_eq!(completion, 0);
    assert_eq!(samples, 4800);
    assert!(
        frames
            .iter()
            .all(|frame| frame.1[0] > 180 && frame.1[1] < 90),
        "{frames:?}"
    );
    send(&mut helper.input, 4, &2u64.to_le_bytes());
    let status = helper.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
}

#[test]
fn the_thumbnail_helper_fails_an_unknown_codec() {
    let Some(bytes) = fixture("unknown.avi", Codecs::UnknownTag) else {
        return;
    };
    let (reply, _) = thumbnail(&bytes, 0);
    assert!(
        matches!(reply, Message::Failed),
        "an unknown codec produced a frame"
    );
}

// MMV2, as the host's video transport speaks it.
fn send(input: &mut ChildStdin, kind: u8, fields: &[u8]) {
    let mut message = b"MMV2".to_vec();
    message.extend_from_slice(&[kind, 0, 0, 0]);
    message.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    message.extend_from_slice(fields);
    input.write_all(&message).unwrap();
    input.flush().unwrap();
}

fn receive(output: &mut ChildStdout) -> (u8, Vec<u8>) {
    let mut header = [0; 12];
    output.read_exact(&mut header).expect("MMV2 header");
    assert_eq!(&header[..4], b"MMV2");
    let length = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let mut payload = vec![0; length];
    output.read_exact(&mut payload).expect("MMV2 payload");
    (header[4], payload)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Answer brokered reads until another message arrives.
fn next(helper: &mut Helper, bytes: &[u8]) -> (u8, Vec<u8>) {
    loop {
        let (kind, payload) = receive(&mut helper.output);
        if kind != 0x82 {
            return (kind, payload);
        }
        let request = u64_at(&payload, 0);
        let offset = u64_at(&payload, 8) as usize;
        let length = u32_at(&payload, 16) as usize;
        let data = &bytes[offset.min(bytes.len())..(offset + length).min(bytes.len())];
        let mut reply = request.to_le_bytes().to_vec();
        reply.push(0);
        reply.extend_from_slice(data);
        send(&mut helper.input, 5, &reply);
    }
}

fn open_player(bytes: &[u8]) -> (Helper, String) {
    let mut helper = Helper::spawn(PLAYER, |input| {
        midnight_worker_sdk::write_policy(
            input,
            midnight_worker_sdk::ResourcePolicy {
                memory_bytes: 512 * 1024 * 1024,
                cpu: midnight_worker_sdk::CpuBudget::NoAdditionalLimit,
            },
        )
    });
    midnight_worker_sdk::read_policy_ready(&mut helper.output)
        .expect("the helper entered its sandbox");
    let ready = format!(
        "ready after {:?}; {}",
        helper.started.elapsed(),
        helper.address_space()
    );
    let mut open = (bytes.len() as u64).to_le_bytes().to_vec();
    open.extend_from_slice(&9u64.to_le_bytes());
    open.extend_from_slice(&160u32.to_le_bytes());
    open.extend_from_slice(&160u32.to_le_bytes());
    send(&mut helper.input, 1, &open);
    (helper, ready)
}

fn pull(
    helper: &mut Helper,
    bytes: &[u8],
    id: u64,
    generation: u64,
    frames: u8,
    pcm: u16,
) -> (Vec<(i64, [u8; 3])>, usize, u8) {
    let mut fields = id.to_le_bytes().to_vec();
    fields.extend_from_slice(&generation.to_le_bytes());
    fields.push(frames);
    fields.extend_from_slice(&pcm.to_le_bytes());
    send(&mut helper.input, 2, &fields);
    let mut seen = Vec::new();
    let mut samples = 0;
    loop {
        let (kind, payload) = next(helper, bytes);
        match kind {
            0x83 => {
                let time = u64_at(&payload, 16) as i64;
                let (width, height) = (u32_at(&payload, 24), u32_at(&payload, 28));
                seen.push((time, centre(width, height, &payload[32..])));
            }
            0x86 => samples += u16::from_le_bytes(payload[24..26].try_into().unwrap()) as usize,
            0x84 => return (seen, samples, payload[16]),
            other => panic!("unexpected MMV2 message {other:#x}: {payload:?}"),
        }
    }
}

#[test]
fn the_playback_helper_serves_frames_audio_and_seeks_inside_its_sandbox() {
    let Some(bytes) = fixture("h264-aac.mp4", Codecs::H264) else {
        eprintln!("skipped: no ffmpeg command to encode the fixture");
        return;
    };
    let (mut helper, ready) = open_player(&bytes);
    eprintln!("playback helper {ready}");
    let (kind, opened) = next(&mut helper, &bytes);
    assert_eq!(kind, 0x81, "expected Opened, got {kind:#x} {opened:?}");
    assert_eq!(u64_at(&opened, 0), 9);
    let duration = u64_at(&opened, 8);
    assert!(
        (3_900_000..=4_200_000).contains(&duration),
        "duration {duration}"
    );
    assert_eq!((u32_at(&opened, 16), u32_at(&opened, 20)), (320, 240));
    assert_eq!(opened[24], 1, "the AAC track is qualified");
    let (frames, samples, completion) = pull(&mut helper, &bytes, 1, 9, 4, 4800);
    assert_eq!(completion, 0);
    assert_eq!(samples, 4800);
    assert_eq!(
        frames.iter().map(|frame| frame.0).collect::<Vec<_>>(),
        [0, 40_000, 80_000, 120_000]
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame.1[0] > 180 && frame.1[1] < 90),
        "{frames:?}"
    );
    let mut seek = 2u64.to_le_bytes().to_vec();
    seek.extend_from_slice(&10u64.to_le_bytes());
    seek.extend_from_slice(&2_500_000u64.to_le_bytes());
    send(&mut helper.input, 3, &seek);
    let (kind, completed) = next(&mut helper, &bytes);
    assert_eq!((kind, completed[16]), (0x84, 0), "seek completion");
    let (frames, _, _) = pull(&mut helper, &bytes, 3, 10, 2, 0);
    assert_eq!(frames.first().map(|frame| frame.0), Some(2_520_000));
    assert!(
        frames
            .iter()
            .all(|frame| frame.1[2] > 180 && frame.1[0] < 90),
        "{frames:?}"
    );
    send(&mut helper.input, 4, &4u64.to_le_bytes());
    let status = helper.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
}

#[test]
fn the_playback_helper_reports_a_missing_decoder_as_cause_five() {
    let Some(bytes) = fixture("unknown.avi", Codecs::UnknownTag) else {
        return;
    };
    let (mut helper, _) = open_player(&bytes);
    let (kind, failed) = next(&mut helper, &bytes);
    assert_eq!(kind, 0x85, "expected Failed, got {kind:#x}");
    assert_eq!(failed.len(), 19);
    let (cause, codec, tried) = (failed[16], failed[17], failed[18]);
    assert_eq!(
        (cause, codec),
        (5, 0),
        "an unknown codec is a missing decoder"
    );
    // Whatever this system has installed (distribution FFmpeg = 2, GStreamer = 4) was tried.
    assert_eq!(tried & !0x06, 0, "tried {tried:#x}");
}

/// The dynamic section of an ELF64 little-endian executable: NEEDED, RPATH and RUNPATH.
fn dynamic(path: &str) -> (Vec<String>, Vec<String>) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        &bytes[..6],
        b"\x7fELF\x02\x01",
        "an ELF64 little-endian file"
    );
    let read = |at: usize, size: usize| -> u64 {
        let mut value = [0u8; 8];
        value[..size].copy_from_slice(&bytes[at..at + size]);
        u64::from_le_bytes(value)
    };
    let (phoff, phentsize, phnum) = (
        read(32, 8) as usize,
        read(54, 2) as usize,
        read(56, 2) as usize,
    );
    let headers: Vec<(u64, u64, u64, u64)> = (0..phnum)
        .map(|index| {
            let at = phoff + index * phentsize;
            (
                read(at, 4),
                read(at + 8, 8),
                read(at + 16, 8),
                read(at + 32, 8),
            )
        })
        .collect();
    let to_offset = |address: u64| {
        headers
            .iter()
            .find(|(kind, _, vaddr, filesz)| {
                *kind == 1 && (*vaddr..vaddr + filesz).contains(&address)
            })
            .map(|(_, offset, vaddr, _)| (address - vaddr + offset) as usize)
            .expect("address inside a loaded segment")
    };
    let (_, dynamic_offset, _, dynamic_size) = *headers
        .iter()
        .find(|header| header.0 == 2)
        .expect("PT_DYNAMIC");
    let entries: Vec<(u64, u64)> = (0..dynamic_size as usize / 16)
        .map(|index| {
            (
                read(dynamic_offset as usize + index * 16, 8),
                read(dynamic_offset as usize + index * 16 + 8, 8),
            )
        })
        .take_while(|entry| entry.0 != 0)
        .collect();
    let strings = to_offset(
        entries
            .iter()
            .find(|entry| entry.0 == 5)
            .expect("DT_STRTAB")
            .1,
    );
    let string = |offset: u64| {
        let start = strings + offset as usize;
        let end = start + bytes[start..].iter().position(|byte| *byte == 0).unwrap();
        String::from_utf8_lossy(&bytes[start..end]).into_owned()
    };
    let needed = entries
        .iter()
        .filter(|entry| entry.0 == 1)
        .map(|entry| string(entry.1))
        .collect();
    let paths = entries
        .iter()
        .filter(|entry| entry.0 == 15 || entry.0 == 29)
        .map(|entry| string(entry.1))
        .collect();
    (needed, paths)
}

#[test]
fn the_system_helpers_link_no_ffmpeg_or_gstreamer_and_have_no_library_path() {
    for executable in [THUMBNAILER, PLAYER] {
        let (needed, paths) = dynamic(executable);
        assert!(
            needed.iter().all(
                |name| !["libav", "libsw", "libgst", "libglib", "libgobject"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            ),
            "{executable} needs {needed:?}"
        );
        // A RUNPATH beside the bundled FFmpeg would let dlopen find its libavcodec.so.63.
        assert!(paths.is_empty(), "{executable} has library paths {paths:?}");
    }
}

#[test]
fn startup_within_the_host_deadlines() {
    let Some(bytes) = fixture("h264-aac.mp4", Codecs::H264) else {
        return;
    };
    let started = Instant::now();
    let (reply, ready) = thumbnail(&bytes, 0);
    assert!(matches!(reply, Message::Frame(_)));
    eprintln!("one thumbnail in {:?} ({ready})", started.elapsed());
    // The thumbnail host allows ten seconds per decode, startup included.
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// A 4K H.264 frame decoded through each family within the 512 MiB address-space limit the
/// thumbnail host gives its helpers (the libraries take about half of it).
#[test]
fn a_4k_frame_fits_the_helpers_memory_policy() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let folder =
        std::env::temp_dir().join(format!("midnight-system-helpers-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let output = folder.join("uhd.mp4");
    let status = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("testsrc2=size=3840x2160:rate=25:duration=2")
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "25",
        ])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&output).unwrap();
    let started = Instant::now();
    let (reply, ready) = thumbnail(&bytes, 5);
    eprintln!("4K thumbnail in {:?} ({ready})", started.elapsed());
    let Message::Frame(frame) = reply else {
        panic!("the 4K frame did not decode within the memory policy");
    };
    assert_eq!((frame.width, frame.height), (64, 36));
}
