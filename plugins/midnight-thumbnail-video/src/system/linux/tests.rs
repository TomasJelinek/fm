//! Tests of the Linux platform decoders against the libraries this system has installed.
//!
//! Every test that loads a decoder family runs in a forked child process: a process maps
//! one FFmpeg series and initializes GStreamer once, exactly like a helper, and a child can
//! also enter the real worker sandbox. Fixtures are encoded at test time with the system's
//! `ffmpeg` command; nothing binary is committed. A test whose library or encoder is absent
//! says so and passes vacuously only for that part (the output names what was skipped).
//!
//! Optional private FFmpeg builds for the version-dispatch tests: set
//! `MIDNIGHT_TEST_FFMPEG_DIRS` to folders that each hold one release's five shared libraries
//! (see the plugin README, "Distribution FFmpeg and GStreamer").
use super::{
    Decoders, Options, Search, ffmpeg, gstreamer,
    library::{self, Trust},
};
use crate::{
    media::{AudioTrack, Codec, NoDecoder, OutputBounds, Tried},
    playback_worker::{PlaybackDecoder, Sources},
};
use midnight_thumbnail_sdk::{ReadSeek, api};
use std::{
    io::{self, Cursor, Read},
    os::fd::RawFd,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
};

// ---------------------------------------------------------------------------------------
// Child processes

/// Where a child writes its result: the pipe, or stdout once the sandbox closed the pipe.
struct Report {
    fd: RawFd,
}

impl Report {
    /// Before entering the sandbox, which closes every descriptor above stderr: move the
    /// result pipe onto stdout.
    fn keep_through_lockdown(&mut self) {
        // SAFETY: dup2 of our own pipe end onto descriptor 1 in this child.
        unsafe { libc::dup2(self.fd, 1) };
        self.fd = 1;
    }
}

/// Run `body` in a forked child and return what it reported. Forks are serialized: a fork
/// while another test thread loads libraries could copy a held loader lock.
fn in_child(body: impl FnOnce(&mut Report) -> Result<String, String>) -> Result<String, String> {
    let _fork = library::LOADER_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut pipe = [0; 2];
    // SAFETY: a new pipe; both ends are closed below in each process.
    assert_eq!(
        unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    // SAFETY: the child only runs `body` on this thread and exits without unwinding.
    let child = unsafe { libc::fork() };
    assert!(child >= 0, "fork failed");
    if child == 0 {
        // SAFETY: the child's copy of the read end.
        unsafe { libc::close(pipe[0]) };
        let mut report = Report { fd: pipe[1] };
        let result = catch_unwind(AssertUnwindSafe(|| body(&mut report))).unwrap_or_else(|panic| {
            let text = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                .unwrap_or_default();
            Err(format!("panicked: {text}"))
        });
        let text = match result {
            Ok(text) => format!("ok:{text}"),
            Err(text) => format!("err:{text}"),
        };
        let mut bytes = text.as_bytes();
        while !bytes.is_empty() {
            // SAFETY: writing our own buffer to our own descriptor.
            let written = unsafe { libc::write(report.fd, bytes.as_ptr().cast(), bytes.len()) };
            if written <= 0 {
                break;
            }
            bytes = &bytes[written as usize..];
        }
        // SAFETY: leave without running the parent's destructors or atexit handlers.
        unsafe { libc::_exit(0) };
    }
    // SAFETY: the parent's copy of the write end.
    unsafe { libc::close(pipe[1]) };
    let mut output = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        // SAFETY: reading into our buffer from our pipe end.
        let count = unsafe { libc::read(pipe[0], buffer.as_mut_ptr().cast(), buffer.len()) };
        if count <= 0 {
            break;
        }
        output.extend_from_slice(&buffer[..count as usize]);
    }
    // SAFETY: our pipe end; waiting for our own child.
    let status = unsafe {
        libc::close(pipe[0]);
        let mut status = 0;
        libc::waitpid(child, &mut status, 0);
        status
    };
    if libc::WIFSIGNALED(status) {
        return Err(format!(
            "child ended by signal {} after reporting {:?}",
            libc::WTERMSIG(status),
            String::from_utf8_lossy(&output)
        ));
    }
    let text = String::from_utf8_lossy(&output).into_owned();
    match text.split_once(':') {
        Some(("ok", rest)) => Ok(rest.to_owned()),
        Some(("err", rest)) => Err(rest.to_owned()),
        _ => Err(format!("child reported nothing usable: {text:?}")),
    }
}

fn check(condition: bool, message: impl FnOnce() -> String) -> Result<(), String> {
    if condition { Ok(()) } else { Err(message()) }
}

// ---------------------------------------------------------------------------------------
// Fixtures, encoded at test time

fn tool(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|folder| Path::new(folder).join(name))
        .find(|path| path.is_file())
}

fn scratch() -> PathBuf {
    static FOLDER: OnceLock<PathBuf> = OnceLock::new();
    FOLDER
        .get_or_init(|| {
            let folder =
                std::env::temp_dir().join(format!("midnight-linux-media-{}", std::process::id()));
            std::fs::create_dir_all(&folder).expect("test scratch folder");
            folder
        })
        .clone()
}

/// Four seconds of solid colour: red, green, blue, white, one second each, 25 fps, 320x240.
const COLOURS: &str = "color=c=red:s=320x240:r=25:d=1[a];color=c=lime:s=320x240:r=25:d=1[b];\
color=c=blue:s=320x240:r=25:d=1[c];color=c=white:s=320x240:r=25:d=1[d];\
[a][b][c][d]concat=n=4:v=1:a=0[v]";

/// Encode `name` with the system's ffmpeg, once per test process; `None` without ffmpeg or
/// the encoder.
fn fixture(name: &str) -> Option<Vec<u8>> {
    static MADE: Mutex<Vec<(String, Option<Vec<u8>>)>> = Mutex::new(Vec::new());
    let mut made = MADE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, bytes)) = made.iter().find(|(made, _)| made == name) {
        return bytes.clone();
    }
    let bytes = encode(name);
    if bytes.is_none() {
        eprintln!("fixture {name} unavailable: no ffmpeg command or encoder on this system");
    }
    made.push((name.to_owned(), bytes.clone()));
    bytes
}

fn encode(name: &str) -> Option<Vec<u8>> {
    let ffmpeg = tool("ffmpeg")?;
    let output = scratch().join(name);
    let sine = "sine=frequency=440:sample_rate=44100:duration=4";
    let colours = [
        "-f",
        "lavfi",
        "-i",
        sine,
        "-filter_complex",
        COLOURS,
        "-map",
        "[v]",
        "-map",
        "0:a",
    ];
    let mut arguments: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| arguments.extend(items.iter().map(|item| (*item).to_owned()));
    match name {
        "h264-aac.mp4" => {
            push(&colours);
            push(&[
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
        "h264-aac-moov-last.mp4" => {
            push(&colours);
            push(&[
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "25", "-c:a", "aac",
            ]);
        }
        "hevc-aac.mkv" => {
            push(&colours);
            push(&[
                "-c:v",
                "libx265",
                "-x265-params",
                "log-level=none",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "25",
                "-c:a",
                "aac",
            ]);
        }
        "mpeg4-mp3.avi" => {
            push(&colours);
            push(&["-c:v", "mpeg4", "-g", "25", "-c:a", "libmp3lame"]);
        }
        // Windows Media Video 8 and Audio 2, and version 7 and 1, in ASF: the containers
        // of `.wmv` and `.asf` files. FFmpeg encodes no WMV 9 or VC-1, which it decodes.
        "wmv2-wmav2.wmv" => {
            push(&colours);
            push(&["-c:v", "wmv2", "-g", "25", "-c:a", "wmav2"]);
        }
        "wmv1-wmav1.asf" => {
            push(&colours);
            push(&["-c:v", "wmv1", "-g", "25", "-c:a", "wmav1", "-f", "asf"]);
        }
        "rotated.mp4" => {
            let base = scratch().join("rotation-base.mp4");
            let status = Command::new(&ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                ])
                .arg("testsrc=size=320x240:rate=25:duration=1")
                .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
                .arg(&base)
                .status()
                .ok()?;
            if !status.success() {
                return None;
            }
            let base = base.to_string_lossy().into_owned();
            push(&["-display_rotation", "90", "-i", &base, "-c", "copy"]);
        }
        _ => return None,
    }
    let status = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(&arguments)
        .arg(&output)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    std::fs::read(&output).ok()
}

/// The MPEG-4 AVI with an unknown codec tag (no decoder anywhere knows it).
fn unknown_codec() -> Option<Vec<u8>> {
    let mut bytes = fixture("mpeg4-mp3.avi")?;
    let mut patched = 0;
    for tag in [*b"FMP4", *b"fmp4"] {
        let mut index = 0;
        while let Some(found) = bytes[index..].windows(4).position(|window| window == tag) {
            let at = index + found;
            bytes[at..at + 4].copy_from_slice(b"ZQ9X");
            index = at + 4;
            patched += 1;
        }
    }
    (patched > 0).then_some(bytes)
}

// ---------------------------------------------------------------------------------------
// Decoder harness

fn options(ffmpeg: Search, gstreamer: Search) -> Options {
    Options {
        ffmpeg,
        gstreamer,
        trust: Trust::ROOT,
    }
}

fn request(bytes: &[u8], frame: u8, edge: u32) -> api::Request {
    api::Request {
        id: 7,
        source_length: bytes.len() as u64,
        width: edge,
        height: edge,
        frame,
    }
}

fn thumbnail(decoders: &Decoders, bytes: &[u8], frame: u8, edge: u32) -> io::Result<api::Frame> {
    decoders.thumbnail(&request(bytes, frame, edge), &mut Cursor::new(bytes))
}

/// The dominant colour of an RGBA picture's centre: r, g, b or w (white), or '?'.
fn colour(width: u32, height: u32, rgba: &[u8]) -> char {
    let at = ((height / 2 * width + width / 2) * 4) as usize;
    let [r, g, b] = [rgba[at], rgba[at + 1], rgba[at + 2]];
    match (r > 180, g > 180, b > 180) {
        (true, true, true) => 'w',
        (true, false, false) if g < 90 && b < 90 => 'r',
        (false, true, false) if r < 90 && b < 90 => 'g',
        (false, false, true) if r < 90 && g < 90 => 'b',
        _ => '?',
    }
}

struct Memory(Vec<u8>);

impl Sources for Memory {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }

    fn cursor(&self) -> Box<dyn ReadSeek + '_> {
        Box::new(Cursor::new(&self.0[..]))
    }
}

/// What a playback session saw, for comparison between the families.
#[derive(Debug, Default)]
struct Session {
    duration: Option<u64>,
    size: (u32, u32),
    audio: Option<AudioTrack>,
    /// (time in ms, colour) of the first frames, then of the first frames after a 2.5 s seek.
    frames: Vec<(i64, char)>,
    after_seek: Vec<(i64, char)>,
    pcm_frames: u64,
    pcm_peak: i16,
    pcm_after_seek: Option<u64>,
}

fn play(decoders: &Decoders, bytes: Vec<u8>) -> io::Result<Session> {
    let sources = Memory(bytes);
    let mut seen = Session::default();
    decoders.playback(
        &sources,
        OutputBounds {
            width: 160,
            height: 160,
        },
        &mut |decoder: &mut dyn PlaybackDecoder| {
            let metadata = decoder.metadata();
            seen.duration = metadata.duration_micros;
            seen.size = (metadata.width, metadata.height);
            seen.audio = Some(decoder.audio_track());
            for _ in 0..30 {
                let Some(frame) = decoder.next_frame()? else {
                    break;
                };
                seen.frames.push((
                    frame.presentation_micros / 1000,
                    colour(frame.width, frame.height, &frame.rgba),
                ));
            }
            if decoder.audio_track() == AudioTrack::Qualified {
                for _ in 0..10 {
                    let Some(block) = decoder.pcm(4800)? else {
                        break;
                    };
                    seen.pcm_frames += (block.samples.len() / 2) as u64;
                    seen.pcm_peak = seen.pcm_peak.max(
                        block
                            .samples
                            .iter()
                            .map(|s| s.saturating_abs())
                            .max()
                            .unwrap_or(0),
                    );
                }
            }
            decoder.seek(2_500_000)?;
            for _ in 0..5 {
                let Some(frame) = decoder.next_frame()? else {
                    break;
                };
                seen.after_seek.push((
                    frame.presentation_micros / 1000,
                    colour(frame.width, frame.height, &frame.rgba),
                ));
            }
            if decoder.audio_track() == AudioTrack::Qualified {
                seen.pcm_after_seek = decoder.pcm(4800)?.map(|block| block.first_frame);
            }
            Ok(())
        },
    )?;
    Ok(seen)
}

/// Checks shared by both families on the four-colour H.264/AAC fixture.
fn check_four_colour_session(seen: &Session) -> Result<(), String> {
    check_four_colours_from(seen, 0)
}

/// The checks of `check_four_colour_session` for a fixture whose first video frame is at
/// `start` milliseconds of the container's timeline: 0 in MP4; one audio frame (46 ms)
/// in ASF as FFmpeg writes it, which starts the video after the first audio frame.
fn check_four_colours_from(seen: &Session, start: i64) -> Result<(), String> {
    check(seen.size == (320, 240), || format!("size {:?}", seen.size))?;
    check(
        seen.duration
            .is_some_and(|duration| (3_900_000..=4_200_000).contains(&duration)),
        || format!("duration {:?}", seen.duration),
    )?;
    check(seen.audio == Some(AudioTrack::Qualified), || {
        format!("audio {:?}", seen.audio)
    })?;
    check(seen.frames.len() == 30, || {
        format!("{} frames", seen.frames.len())
    })?;
    check(seen.frames[0] == (start, 'r'), || {
        format!("first frame {:?}", seen.frames[0])
    })?;
    check(
        seen.frames
            .windows(2)
            .all(|pair| pair[1].0 == pair[0].0 + 40),
        || {
            format!(
                "frame times {:?}",
                seen.frames.iter().map(|frame| frame.0).collect::<Vec<_>>()
            )
        },
    )?;
    check(seen.frames[25] == (start + 1000, 'g'), || {
        format!("frame 25 {:?}", seen.frames[25])
    })?;
    check(seen.pcm_frames == 48_000, || {
        format!("{} PCM frames", seen.pcm_frames)
    })?;
    check(seen.pcm_peak > 2_000, || {
        format!("PCM peak {}", seen.pcm_peak)
    })?;
    // The first frame at or after the 2.5 s target (2520 ms from a start at 0).
    let after = start + (2500 - start + 39) / 40 * 40;
    check(seen.after_seek.first() == Some(&(after, 'b')), || {
        format!("after seek {:?}", seen.after_seek)
    })?;
    check(seen.pcm_after_seek == Some(0), || {
        format!("PCM after seek {:?}", seen.pcm_after_seek)
    })
}

fn describe(error: &io::Error) -> String {
    match NoDecoder::find(error) {
        Some(missing) => format!(
            "NoDecoder(codec {:?}, tried {:#04x})",
            missing.codec, missing.tried.0
        ),
        None => format!("{:?}: {error}", error.kind()),
    }
}

// ---------------------------------------------------------------------------------------
// Tests: the distribution's FFmpeg

#[test]
fn the_distribution_ffmpeg_is_found_by_soname_and_checked_against_its_series() {
    let result = in_child(|_| {
        let loaded =
            ffmpeg::load(&Search::System, Trust::ROOT).map_err(|error| error.to_string())?;
        let versions = loaded.api.versions;
        let mapped = library::ffmpeg_sonames();
        // One series only: every mapped FFmpeg library belongs to the loaded set.
        check(
            mapped.iter().all(|(stem, major)| match stem.as_str() {
                "libavcodec" => *major == versions.avcodec >> 16,
                "libavformat" => *major == versions.avformat >> 16,
                "libavutil" => *major == versions.avutil >> 16,
                "libswscale" => *major == versions.swscale >> 16,
                "libswresample" => *major == versions.swresample >> 16,
                _ => false,
            }),
            || format!("mapped {mapped:?} for {versions}"),
        )?;
        // A second load in the same process is refused rather than mapping another series.
        check(
            matches!(
                ffmpeg::load(&Search::System, Trust::ROOT),
                Err(ffmpeg::Unavailable::Mixed(_))
            ),
            || "a second FFmpeg load was not refused".into(),
        )?;
        Ok(format!("{versions}; mapped {mapped:?}"))
    });
    match result {
        Ok(found) => eprintln!("distribution FFmpeg: {found}"),
        Err(error) if error.contains("no supported FFmpeg") => {
            eprintln!("skipped: no distribution FFmpeg installed ({error})")
        }
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn the_distribution_ffmpeg_decodes_h264_and_aac_thumbnails_and_playback() {
    let Some(bytes) = fixture("h264-aac.mp4") else {
        return;
    };
    let result = in_child(move |_| {
        let decoders = Decoders::load(&options(Search::System, Search::Hidden));
        check(decoders.ffmpeg.is_some(), || {
            "no distribution FFmpeg".into()
        })?;
        let mut colours = String::new();
        for frame in 0..6 {
            let thumbnail =
                thumbnail(&decoders, &bytes, frame, 64).map_err(|error| describe(&error))?;
            check((thumbnail.width, thumbnail.height) == (64, 48), || {
                format!("thumbnail size {}x{}", thumbnail.width, thumbnail.height)
            })?;
            colours.push(colour(thumbnail.width, thumbnail.height, &thumbnail.rgba));
        }
        // 10%, 25%, 40%, 55%, 70% and 85% of four seconds.
        check(colours == "rggbbw", || {
            format!("thumbnail colours {colours}")
        })?;
        let seen = play(&decoders, bytes).map_err(|error| describe(&error))?;
        check_four_colour_session(&seen)?;
        Ok(format!("{colours} {seen:?}"))
    });
    result.unwrap();
}

#[test]
fn hevc_mpeg4_and_mp3_decode_through_the_distribution_ffmpeg() {
    for name in ["hevc-aac.mkv", "mpeg4-mp3.avi"] {
        let Some(bytes) = fixture(name) else { continue };
        let result = in_child(move |_| {
            let decoders = Decoders::load(&options(Search::System, Search::Hidden));
            let frame = thumbnail(&decoders, &bytes, 0, 64).map_err(|error| describe(&error))?;
            check(
                colour(frame.width, frame.height, &frame.rgba) == 'r',
                || "first sample is not red".into(),
            )?;
            let seen = play(&decoders, bytes).map_err(|error| describe(&error))?;
            check(
                seen.audio == Some(AudioTrack::Qualified) && seen.pcm_peak > 2_000,
                || format!("{seen:?}"),
            )?;
            check(
                seen.after_seek
                    .first()
                    .is_some_and(|frame| frame.1 == 'b' && frame.0 >= 2500),
                || format!("{seen:?}"),
            )?;
            Ok(String::new())
        });
        result.unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

/// The extensions that `thumbnailer.json` routes to the video helpers.
fn routed_extensions() -> Vec<String> {
    let manifest = include_str!("../../../thumbnailer.json");
    let list = manifest
        .split_once("\"extensions\"")
        .and_then(|(_, rest)| rest.split_once('['))
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list)
        .expect("thumbnailer.json lists extensions");
    list.split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .collect()
}

#[test]
fn every_routed_extension_has_its_demuxer_in_both_families() {
    let whitelist: Vec<&str> = ffmpeg::FORMATS.to_str().unwrap().split(',').collect();
    let extensions = routed_extensions();
    assert!(extensions.iter().any(|extension| extension == "wmv"));
    for extension in &extensions {
        // FFmpeg's demuxer name and GStreamer's plugin of each extension's container.
        let (demuxer, plugin) = match extension.as_str() {
            "webm" | "mkv" | "mk3d" => ("matroska", "matroska"),
            "avi" => ("avi", "avi"),
            "ogg" | "ogv" => ("ogg", "ogg"),
            "mp4" | "m4v" | "mov" | "qt" | "3gp" | "3g2" => ("mov", "isomp4"),
            "wmv" | "asf" => ("asf", "asf"),
            other => panic!("no container known for the routed extension {other}"),
        };
        assert!(
            whitelist.contains(&demuxer),
            "{extension}: FFmpeg {demuxer}"
        );
        assert!(
            gstreamer::PLUGINS.contains(&plugin),
            "{extension}: GStreamer {plugin}"
        );
    }
}

/// Thumbnails at the six sample positions and a playback session of the four-colour
/// Windows Media fixture, checked as for H.264.
fn decode_windows_media(decoders: &Decoders, bytes: Vec<u8>) -> Result<String, String> {
    let mut colours = String::new();
    for frame in 0..6 {
        let thumbnail = thumbnail(decoders, &bytes, frame, 64).map_err(|error| describe(&error))?;
        check((thumbnail.width, thumbnail.height) == (64, 48), || {
            format!("thumbnail size {}x{}", thumbnail.width, thumbnail.height)
        })?;
        colours.push(colour(thumbnail.width, thumbnail.height, &thumbnail.rgba));
    }
    check(colours == "rggbbw", || {
        format!("thumbnail colours {colours}")
    })?;
    let seen = play(decoders, bytes).map_err(|error| describe(&error))?;
    // FFmpeg's ASF muxer starts the video one Windows Media Audio frame (2048 samples at
    // 44.1 kHz, 46 ms in ASF's millisecond time base) after the audio.
    check_four_colours_from(&seen, 46)?;
    Ok(format!("{colours} {seen:?}"))
}

#[test]
fn windows_media_in_asf_decodes_through_the_distribution_ffmpeg() {
    for name in ["wmv2-wmav2.wmv", "wmv1-wmav1.asf"] {
        let Some(bytes) = fixture(name) else { continue };
        let result = in_child(move |_| {
            let decoders = Decoders::load(&options(Search::System, Search::Hidden));
            check(decoders.ffmpeg.is_some(), || {
                "no distribution FFmpeg".into()
            })?;
            decode_windows_media(&decoders, bytes)
        });
        result.unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

// ---------------------------------------------------------------------------------------
// Tests: GStreamer, with the FFmpeg libraries hidden from the helper

#[test]
fn without_ffmpeg_gstreamer_decodes_windows_media_with_its_asf_demuxer() {
    let Some(bytes) = fixture("wmv2-wmav2.wmv") else {
        return;
    };
    let result = in_child(move |_| {
        let decoders = Decoders::load(&options(Search::Hidden, Search::System));
        let gstreamer = decoders.gstreamer.ok_or("GStreamer is not installed")?;
        let asf = gstreamer.plugins.contains(&"asf");
        let libav = gstreamer.plugins.contains(&"libav");
        if asf && libav {
            return decode_windows_media(&decoders, bytes);
        }
        // Without gst-plugins-ugly's demuxer, or without gst-libav's decoders, the report
        // is precise: a missing plugin for the container, or the codec.
        let error = thumbnail(&decoders, &bytes, 0, 64)
            .err()
            .ok_or("decoded without the plugins")?;
        let codec = if asf { Codec::Vc1 } else { Codec::Unknown };
        check(
            NoDecoder::find(&error)
                == Some(NoDecoder {
                    codec,
                    tried: Tried::GSTREAMER,
                }),
            || describe(&error),
        )?;
        Ok(format!(
            "skipped decoding: asf {asf}, libav {libav} (plugins {:?}, skipped {:?})",
            gstreamer.plugins, gstreamer.skipped
        ))
    });
    match result {
        Ok(outcome) => eprintln!("{outcome}"),
        Err(error) if error.contains("GStreamer is not installed") => eprintln!("skipped: {error}"),
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn without_ffmpeg_gstreamer_decodes_h264_and_aac_thumbnails_and_playback() {
    let Some(bytes) = fixture("h264-aac.mp4") else {
        return;
    };
    let result = in_child(move |_| {
        let decoders = Decoders::load(&options(Search::Hidden, Search::System));
        check(decoders.ffmpeg.is_none(), || "FFmpeg was not hidden".into())?;
        let gstreamer = decoders.gstreamer.ok_or("GStreamer is not installed")?;
        let mut colours = String::new();
        for frame in 0..6 {
            let thumbnail =
                thumbnail(&decoders, &bytes, frame, 64).map_err(|error| describe(&error))?;
            check((thumbnail.width, thumbnail.height) == (64, 48), || {
                format!("thumbnail size {}x{}", thumbnail.width, thumbnail.height)
            })?;
            colours.push(colour(thumbnail.width, thumbnail.height, &thumbnail.rgba));
        }
        check(colours == "rggbbw", || {
            format!("thumbnail colours {colours}")
        })?;
        let seen = play(&decoders, bytes).map_err(|error| describe(&error))?;
        check_four_colour_session(&seen)?;
        Ok(format!("plugins {:?}", gstreamer.plugins))
    });
    match result {
        Err(error) if error.contains("GStreamer is not installed") => eprintln!("skipped: {error}"),
        other => {
            other.unwrap();
        }
    }
}

#[test]
fn gstreamer_seeks_an_mp4_whose_index_follows_the_media() {
    // The demuxer must fetch the index from the end through a byte seek on our source pad,
    // then seek back: push-mode seeking in both directions, on one thread.
    let Some(bytes) = fixture("h264-aac-moov-last.mp4") else {
        return;
    };
    in_child(move |_| {
        let decoders = Decoders::load(&options(Search::Hidden, Search::System));
        let mut colours = String::new();
        for frame in [0, 5] {
            let thumbnail =
                thumbnail(&decoders, &bytes, frame, 64).map_err(|error| describe(&error))?;
            colours.push(colour(thumbnail.width, thumbnail.height, &thumbnail.rgba));
        }
        check(colours == "rw", || format!("colours {colours}"))?;
        Ok(String::new())
    })
    .unwrap();
}

#[test]
fn both_families_orient_a_rotated_video_alike() {
    let Some(bytes) = fixture("rotated.mp4") else {
        return;
    };
    in_child(move |_| {
        let decoders = Decoders::load(&options(Search::System, Search::Hidden));
        let frame = thumbnail(&decoders, &bytes, 0, 64).map_err(|error| describe(&error))?;
        check((frame.width, frame.height) == (48, 64), || {
            format!("FFmpeg {}x{}", frame.width, frame.height)
        })?;
        Ok(String::new())
    })
    .unwrap();
    let rotated = fixture("rotated.mp4").expect("fixture made above");
    in_child(move |_| {
        let decoders = Decoders::load(&options(Search::Hidden, Search::System));
        let frame = thumbnail(&decoders, &rotated, 0, 64).map_err(|error| describe(&error))?;
        check((frame.width, frame.height) == (48, 64), || {
            format!("GStreamer {}x{}", frame.width, frame.height)
        })?;
        Ok(String::new())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------------------
// Tests: precise reports

#[test]
fn a_codec_no_family_decodes_is_reported_with_what_was_tried() {
    let Some(bytes) = unknown_codec() else { return };
    let result = in_child(move |_| {
        let decoders = Decoders::load(&options(Search::System, Search::System));
        let error = thumbnail(&decoders, &bytes, 0, 64)
            .err()
            .ok_or("decoded an unknown codec")?;
        let missing = NoDecoder::find(&error).ok_or_else(|| describe(&error))?;
        let mut expected = Tried::default();
        if decoders.ffmpeg.is_some() {
            expected = expected.union(Tried::DISTRIBUTION_FFMPEG);
        }
        if decoders.gstreamer.is_some() {
            expected = expected.union(Tried::GSTREAMER);
        }
        check(
            missing
                == NoDecoder {
                    codec: Codec::Unknown,
                    tried: expected,
                },
            || format!("{missing:?}"),
        )?;
        let sources = Memory(bytes);
        let error = decoders
            .playback(
                &sources,
                OutputBounds {
                    width: 64,
                    height: 64,
                },
                &mut |_| Ok(()),
            )
            .err()
            .ok_or("playback opened an unknown codec")?;
        check(NoDecoder::find(&error) == Some(missing), || {
            describe(&error)
        })?;
        Ok(String::new())
    });
    result.unwrap();
}

#[test]
fn with_no_family_installed_a_known_codec_is_still_a_missing_decoder() {
    let Some(bytes) = fixture("h264-aac.mp4") else {
        return;
    };
    in_child(move |_| {
        let decoders = Decoders::load(&options(Search::Hidden, Search::Hidden));
        let error = thumbnail(&decoders, &bytes, 0, 64)
            .err()
            .ok_or("decoded without decoders")?;
        // Nothing here could name the codec; the host keeps the bundled helper's name.
        check(
            NoDecoder::find(&error)
                == Some(NoDecoder {
                    codec: Codec::Unknown,
                    tried: Tried::default(),
                }),
            || describe(&error),
        )?;
        Ok(String::new())
    })
    .unwrap();
}

#[test]
fn garbage_is_invalid_data_not_a_missing_decoder() {
    let bytes = b"this is not a video container at all, just text".repeat(64);
    in_child(move |_| {
        let decoders = Decoders::load(&options(Search::System, Search::System));
        if decoders.ffmpeg.is_none() && decoders.gstreamer.is_none() {
            return Ok("no family installed".into());
        }
        let error = thumbnail(&decoders, &bytes, 0, 64)
            .err()
            .ok_or("decoded text")?;
        check(
            error.kind() == io::ErrorKind::InvalidData && NoDecoder::find(&error).is_none(),
            || describe(&error),
        )?;
        Ok(String::new())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------------------
// Tests: the sandbox

#[test]
fn gstreamer_loads_only_listed_plugins_and_never_touches_the_registry() {
    let result = in_child(|_| {
        let home = scratch().join("home");
        std::fs::create_dir_all(&home).map_err(|error| error.to_string())?;
        // SAFETY: the forked child has one thread.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_CACHE_HOME", home.join("cache"));
            std::env::set_var("GST_PLUGIN_PATH", "/tmp");
        }
        let loaded = gstreamer::load(&Search::System, Trust::ROOT, true)
            .map_err(|error| error.to_string())?;
        let disabled = std::env::var("GST_REGISTRY_DISABLE").ok();
        check(disabled.as_deref() == Some("yes"), || {
            format!("GST_REGISTRY_DISABLE {disabled:?}")
        })?;
        check(
            std::env::var("GST_PLUGIN_PATH").ok().as_deref() == Some(""),
            || "user plugin path kept".into(),
        )?;
        check(
            std::fs::read_dir(&home)
                .map(|entries| entries.count())
                .unwrap_or(0)
                == 0,
            || "something was written below HOME".into(),
        )?;
        check(!loaded.plugins.is_empty(), || "no plugin loaded".into())?;
        // Installed plugins that are not on the list stay unloaded: no network source (the
        // ASF plugin's RTSP extension has no RTSP source to extend), no VA-API decoder.
        for element in [
            c"souphttpsrc",
            c"rtspsrc",
            c"udpsrc",
            c"vah264dec",
            c"v4l2src",
            c"playbin",
        ] {
            // SAFETY: a factory lookup on the loaded GStreamer.
            check(!gstreamer::has_element(loaded, element), || {
                format!("{element:?} is available")
            })?;
        }
        check(loaded.incomplete.is_empty(), || {
            format!("missing elements {:?}", loaded.incomplete)
        })?;
        Ok(format!(
            "GStreamer {:?} from {}; loaded {:?}; skipped {:?}",
            loaded.version,
            loaded.plugin_folder.display(),
            loaded.plugins,
            loaded.skipped
        ))
    });
    match result {
        Ok(found) => eprintln!("{found}"),
        Err(error) if error.contains("not installed") => eprintln!("skipped: {error}"),
        Err(error) => panic!("{error}"),
    }
}

#[cfg(feature = "playback-worker")]
fn lockdown(report: &mut Report) -> Result<(), String> {
    report.keep_through_lockdown();
    let mut policy = Vec::new();
    midnight_worker_sdk::write_policy(
        &mut policy,
        midnight_worker_sdk::ResourcePolicy {
            memory_bytes: 1024 * 1024 * 1024,
            cpu: midnight_worker_sdk::CpuBudget::Seconds(60),
        },
    )
    .map_err(|error| error.to_string())?;
    midnight_worker_sdk::enter(&mut Cursor::new(policy), &mut Vec::new())
        .map_err(|error| error.to_string())?;
    // Proof that the sandbox is in force: no file opens any more.
    check(std::fs::File::open("/etc/hostname").is_err(), || {
        "a file opened after lockdown".into()
    })
}

#[cfg(feature = "playback-worker")]
#[test]
fn both_families_decode_inside_the_worker_sandbox() {
    let Some(bytes) = fixture("h264-aac.mp4") else {
        return;
    };
    for (name, ffmpeg_search) in [("FFmpeg", true), ("GStreamer", false)] {
        let bytes = bytes.clone();
        let result = in_child(move |report| {
            let decoders = if ffmpeg_search {
                Decoders::load(&options(Search::System, Search::Hidden))
            } else {
                Decoders::load(&options(Search::Hidden, Search::System))
            };
            if decoders.ffmpeg.is_none() && decoders.gstreamer.is_none() {
                return Ok("family not installed".into());
            }
            lockdown(report)?;
            let frame = thumbnail(&decoders, &bytes, 3, 64).map_err(|error| describe(&error))?;
            check(
                colour(frame.width, frame.height, &frame.rgba) == 'b',
                || "sample 3 is not blue".into(),
            )?;
            let seen = play(&decoders, bytes).map_err(|error| describe(&error))?;
            check_four_colour_session(&seen)?;
            Ok(String::new())
        });
        result.unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn every_mapped_object_passes_the_read_rules_after_loading_both_families() {
    in_child(|_| {
        let decoders = Decoders::load(&Options::SYSTEM);
        library::audit(Trust::ROOT).map_err(|error| error.to_string())?;
        let objects = library::loaded_objects();
        Ok(format!(
            "ffmpeg {} gstreamer {}; {} objects mapped",
            decoders.ffmpeg.is_some(),
            decoders.gstreamer.is_some(),
            objects.len()
        ))
    })
    .map(|summary| eprintln!("{summary}"))
    .unwrap();
}

// ---------------------------------------------------------------------------------------
// Tests: version dispatch against private FFmpeg builds of other series

fn private_builds() -> Vec<PathBuf> {
    std::env::var("MIDNIGHT_TEST_FFMPEG_DIRS")
        .map(|dirs| {
            dirs.split(':')
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn test_trust() -> Trust {
    Trust {
        // SAFETY: getuid has no preconditions.
        also_owner: Some(unsafe { libc::getuid() }),
    }
}

#[test]
fn each_private_build_loads_as_its_own_series_and_decodes() {
    let builds = private_builds();
    if builds.is_empty() {
        eprintln!("skipped: MIDNIGHT_TEST_FFMPEG_DIRS names no private FFmpeg build");
        return;
    }
    let Some(bytes) = fixture("h264-aac.mp4") else {
        return;
    };
    for folder in builds {
        let bytes = bytes.clone();
        let folder_text = folder.display().to_string();
        let result = in_child(move |_| {
            let loaded = ffmpeg::load(&Search::Folder(folder.clone()), test_trust())
                .map_err(|error| error.to_string())?;
            let versions = loaded.api.versions;
            let mapped = library::ffmpeg_sonames();
            check(mapped.len() == 5, || format!("mapped {mapped:?}"))?;
            let decoders = Decoders {
                ffmpeg: Some(loaded),
                gstreamer: None,
            };
            // Private builds carry the native H.264 and AAC decoders (see the recipe).
            let frame = thumbnail(&decoders, &bytes, 3, 64).map_err(|error| describe(&error))?;
            check(
                colour(frame.width, frame.height, &frame.rgba) == 'b',
                || "sample 3 is not blue".into(),
            )?;
            let seen = play(&decoders, bytes).map_err(|error| describe(&error))?;
            check_four_colour_session(&seen)?;
            Ok(format!("{versions}"))
        });
        match result {
            Ok(versions) => eprintln!("{folder_text}: {versions}"),
            Err(error) => panic!("{folder_text}: {error}"),
        }
    }
}

#[test]
fn a_codec_missing_from_the_distribution_ffmpeg_falls_through_to_gstreamer() {
    // MIDNIGHT_TEST_FFMPEG_NO_HEVC names a private build without the HEVC and MPEG-4 Part 2
    // decoders, like a distribution FFmpeg built without patent-encumbered codecs.
    let Some(folder) = std::env::var_os("MIDNIGHT_TEST_FFMPEG_NO_HEVC").map(PathBuf::from) else {
        eprintln!("skipped: MIDNIGHT_TEST_FFMPEG_NO_HEVC is not set");
        return;
    };
    let (Some(hevc), Some(mpeg4)) = (fixture("hevc-aac.mkv"), fixture("mpeg4-mp3.avi")) else {
        return;
    };
    let result = in_child(move |_| {
        let decoders = Decoders::load(&Options {
            ffmpeg: Search::Folder(folder),
            gstreamer: Search::System,
            trust: test_trust(),
        });
        check(decoders.ffmpeg.is_some(), || {
            "private build did not load".into()
        })?;
        let gstreamer = decoders.gstreamer.ok_or("GStreamer is not installed")?;
        // gst-libav would map a second FFmpeg: never loaded beside the distribution's.
        check(!gstreamer.plugins.contains(&"libav"), || {
            "gst-libav loaded beside FFmpeg".into()
        })?;
        let mut outcome = String::new();
        // HEVC: GStreamer's libde265 decoder plays it when installed.
        match thumbnail(&decoders, &hevc, 0, 64) {
            Ok(frame) => {
                check(
                    colour(frame.width, frame.height, &frame.rgba) == 'r',
                    || "HEVC not red".into(),
                )?;
                outcome.push_str("hevc decoded by GStreamer; ");
            }
            Err(error) => {
                check(
                    NoDecoder::find(&error)
                        == Some(NoDecoder {
                            codec: Codec::Hevc,
                            tried: Tried::DISTRIBUTION_FFMPEG.union(Tried::GSTREAMER),
                        }),
                    || describe(&error),
                )?;
                outcome.push_str("hevc reported missing; ");
            }
        }
        // MPEG-4 Part 2: no GStreamer decoder without gst-libav, so it is named precisely.
        let error = thumbnail(&decoders, &mpeg4, 0, 64)
            .err()
            .ok_or("MPEG-4 decoded")?;
        check(
            NoDecoder::find(&error)
                == Some(NoDecoder {
                    codec: Codec::Mpeg4Part2,
                    tried: Tried::DISTRIBUTION_FFMPEG.union(Tried::GSTREAMER),
                }),
            || describe(&error),
        )?;
        Ok(outcome)
    });
    match result {
        Ok(outcome) => eprintln!("{outcome}"),
        Err(error) if error.contains("GStreamer is not installed") => eprintln!("skipped: {error}"),
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn a_source_read_failure_is_reported_as_itself() {
    let Some(bytes) = fixture("h264-aac.mp4") else {
        return;
    };
    struct Failing(Cursor<Vec<u8>>, u64);
    impl Read for Failing {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let position = self.0.position();
            if position >= self.1 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "broker denied read",
                ));
            }
            // Never return bytes past the failure point, however much is asked for.
            let allowed = (self.1 - position).min(buffer.len() as u64) as usize;
            self.0.read(&mut buffer[..allowed])
        }
    }
    impl io::Seek for Failing {
        fn seek(&mut self, from: io::SeekFrom) -> io::Result<u64> {
            self.0.seek(from)
        }
    }
    for hide_ffmpeg in [false, true] {
        let bytes = bytes.clone();
        in_child(move |_| {
            let decoders = if hide_ffmpeg {
                Decoders::load(&options(Search::Hidden, Search::System))
            } else {
                Decoders::load(&options(Search::System, Search::Hidden))
            };
            let length = bytes.len() as u64;
            let mut source = Failing(Cursor::new(bytes), length / 2);
            let request = api::Request {
                id: 1,
                source_length: length,
                width: 64,
                height: 64,
                frame: 5,
            };
            let error = decoders
                .thumbnail(&request, &mut source)
                .err()
                .ok_or("decoded past a failed read")?;
            check(error.kind() == io::ErrorKind::PermissionDenied, || {
                describe(&error)
            })?;
            Ok(String::new())
        })
        .unwrap_or_else(|error| panic!("hide FFmpeg {hide_ffmpeg}: {error}"));
    }
}

#[test]
fn gstreamer_constants_and_struct_offsets_match_the_installed_library() {
    let result = in_child(|_| {
        let loaded = gstreamer::load(&Search::System, Trust::ROOT, true)
            .map_err(|error| error.to_string())?;
        gstreamer::check_abi(loaded)
    });
    match result {
        Ok(summary) => eprintln!("{summary}"),
        Err(error) if error.contains("not installed") => eprintln!("skipped: {error}"),
        Err(error) => panic!("{error}"),
    }
}
