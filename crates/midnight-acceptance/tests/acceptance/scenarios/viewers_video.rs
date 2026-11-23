//! Video through the installed thumbnailers package, as a user's installation runs it:
//! the thumbnail and playback helpers start from `thumbnailers/` beside the
//! application, next to their FFmpeg libraries, in the restricted workers' sandbox.
//! `.mp4` files are generated here: a plain one and, on Windows, an NTFS-compressed
//! copy, and an H.264 + AAC movie (on Windows written by Windows' own encoders and also
//! NTFS-compressed; on Linux by the host's `ffmpeg` with `libx264`). For each, its own
//! tile in the thumbnail view (Alt+5) must show the video's colour, not the fallback
//! icon, and F3 must open the video viewer on its first frame; a click at three quarters
//! of the position slider shows the later frames, Home on the slider the first ones
//! again, and the Play button plays to the end. A helper that cannot start (a library it
//! cannot load) fails the scenario with the viewer's own sentence, such as "The video
//! helper stopped: a required library is missing or cannot be read
//! (STATUS_DLL_NOT_FOUND, 0xC0000135)"; with dialogs suppressed for every helper, no
//! system dialog can stall it. Cases this machine cannot make are reported as skipped.
//!
//! The plain videos hold uncompressed 64 x 48 RGB frames in an MP4 container, which the
//! bundled FFmpeg profile reads (rawvideo in the MOV/MP4 demuxer). That profile has no
//! H.264 or AAC decoder: the H.264 movie reaches the system's decoders, on Windows
//! through the Media Foundation helpers and on Linux through the distribution's FFmpeg
//! libraries, then GStreamer, after the bundled helpers report that they have no decoder
//! for it. On Windows the viewer opens an `.mp4` with the Media Foundation helper first,
//! so there the plain videos reach the bundled helper after that one cannot open them.
//! The report notes each video's time from F3 to its first frame. The first ten frames
//! are magenta, the last ten green: both colours read the same as RGB or BGR, and stay
//! far apart after lossy compression and the decoders' colour conversion.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect, panel, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, Viewing},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{LogicalPosition, platform::Key};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(8 * 60);
const THUMBNAIL_LIMIT: Duration = Duration::from_secs(45);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
const STEP_LIMIT: Duration = Duration::from_secs(20);
const WIDTH: u16 = 64;
const HEIGHT: u16 = 48;
/// Frames per second; the videos last two seconds.
const RATE: u32 = 10;
const FIRST: [u8; 3] = [200, 40, 200];
const LATER: [u8; 3] = [40, 200, 40];
/// How far a shown pixel may be from a frame's colour: H.264, limited-range YCbCr and the
/// BT.709-to-sRGB transfer move both colours by up to about 20 per channel; the two
/// colours stay 160 apart.
const TOLERANCE: u8 = 32;
const PLAIN: &str = "plain.mp4";
const PACKED: &str = "packed.mp4";
const H264: &str = "h264.mp4";
const H264_PACKED: &str = "h264-packed.mp4";

#[cfg(windows)]
#[path = "viewers_video_h264.rs"]
mod h264;

pub fn video(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn run(context: &mut Context) -> Result<(), String> {
    let staged = std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()
                .map(|folder| folder.join("thumbnailers"))
        })
        .is_some_and(|folder| folder.join("midnight-thumbnail-video.json").is_file());
    if !staged {
        return Err(
            "SKIP: the staged installation has no video thumbnailer package; build the \
             thumbnailers first (build.bat or ./build.sh) or name a folder holding \
             thumbnailers/ in MIDNIGHT_ACCEPTANCE_PLUGINS"
                .into(),
        );
    }
    let folder = context.work.join("video");
    std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let bytes = raw_mp4();
    for name in [PLAIN, PACKED] {
        std::fs::write(folder.join(name), &bytes)
            .map_err(|error| format!("{}: {error}", folder.join(name).display()))?;
    }
    let failures = Failures::default();
    let mut names = vec![PLAIN];
    match compress(&folder.join(PACKED)) {
        Ok(()) => names.push(PACKED),
        Err(reason) => failures.skip("an NTFS-compressed video", &reason),
    }
    match h264(&folder) {
        Ok(()) => {
            names.push(H264);
            match compress(&folder.join(H264_PACKED)) {
                Ok(()) => names.push(H264_PACKED),
                Err(reason) => failures.skip("an NTFS-compressed H.264 movie", &reason),
            }
        }
        Err(reason) => failures.skip("an H.264 + AAC movie", &reason),
    }
    // Only the cases made above: the copies that could not be compressed or made go.
    for name in [PACKED, H264, H264_PACKED] {
        if !names.contains(&name) {
            let _ = std::fs::remove_file(folder.join(name));
        }
    }
    let mut config = Config::new(&folder, &folder);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Alt], "5");
    failures.check("thumbnails", thumbnails(&mut app, context, &names));
    // Back to the Detailed list (Alt+3) for F3: panel::select_row finds a row by its
    // accessible label, which for an NTFS-compressed file adds " · Filesystem compressed",
    // and then falls back to Home and Down, which in the thumbnail grid move by whole
    // grid rows.
    input::chord(app.slint(), &[Key::Alt], "3");
    app.settle();
    for name in &names {
        failures.check(&format!("F3 on {name}"), playback(&mut app, context, name));
    }
    app.shutdown()?;
    failures.finish(context)
}

/// Each video's own tile shows its first colour: a decoded frame, never the fallback
/// icon, which has neither colour. Tiles are told apart by their grid cells.
fn thumbnails(app: &mut App, context: &mut Context, names: &[&str]) -> Result<(), String> {
    let started = Instant::now();
    let counts = loop {
        app.settle();
        let counts: Vec<(&str, Option<usize>)> = names
            .iter()
            .map(|name| {
                (
                    *name,
                    tile(app, name).map(|tile| tile.count_near(FIRST, TOLERANCE)),
                )
            })
            .collect();
        let done = counts
            .iter()
            .all(|(_, shown)| shown.is_some_and(|shown| shown >= TILE_PIXELS));
        if done || started.elapsed() >= THUMBNAIL_LIMIT {
            break counts;
        }
        app.idle(Duration::from_millis(40));
    };
    app.screenshot(
        context,
        "video-thumbnails",
        &format!(
            "Alt+5 with {}: pixels of the first colour per tile {counts:?} after {} ms",
            names.join(" and "),
            started.elapsed().as_millis()
        ),
    );
    let missing: Vec<String> = counts
        .iter()
        .filter(|(_, shown)| !shown.is_some_and(|shown| shown >= TILE_PIXELS))
        .map(|(name, shown)| match shown {
            Some(shown) => format!("{name} ({shown} pixels)"),
            None => format!("{name} (no tile in view)"),
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let notices = support::plugins(app)
        .map(|plugins| plugins.thumbnail_log.borrow().notices.clone())
        .unwrap_or_default();
    Err(format!(
        "after {THUMBNAIL_LIMIT:?} these tiles show a fallback icon instead of a frame (fewer \
         than {TILE_PIXELS} pixels of the video's colour): {}; thumbnail notices {notices:?}",
        missing.join(", ")
    ))
}

/// A tile's frame shows the video's colour over at least this many pixels (a 96-pixel
/// tile shows the 64 x 48 frame over about 6,900).
const TILE_PIXELS: usize = 600;

/// The rendered grid cell of `name` in the left panel's thumbnail view.
fn tile(app: &App, name: &str) -> Option<Canvas> {
    let (x, y, width, height) = inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .find(|(label, _)| panel::is_row_label(label, name))
        .map(|(_, item)| inspect::rect(&item))?;
    let (render_width, _, pixels) = app::render(&app.main);
    Some(Canvas::crop(
        &pixels,
        render_width,
        x.max(0.0) as usize,
        y.max(0.0) as usize,
        width.max(0.0) as usize,
        height.max(0.0) as usize,
    ))
}

/// F3 opens the video viewer on its first frame; the position slider and the Play
/// button work by pointer and keyboard.
fn playback(app: &mut App, context: &mut Context, name: &str) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, name)?;
    let pressed = Instant::now();
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let result = (|| {
        viewing.result.clone()?;
        wait_frame(&viewing, app, "the first frame", FIRST, |state| {
            state.can_play_pause && state.can_seek
        })?;
        let first = pressed.elapsed();
        context.note(format!(
            "{name}: F3 to the first frame took {} ms",
            first.as_millis()
        ));
        support::save_canvas(
            context,
            &viewing.canvas(),
            &format!("video-{name}-first"),
            &format!("{name}: the first frame, {} ms after F3", first.as_millis()),
        );
        // A click at three quarters of the position slider.
        let slider = inspect::find(viewing.slint(), AccessibleRole::Slider, "Playback position")
            .ok_or("no Playback position slider")?;
        let (x, y, width, height) = inspect::rect(&slider);
        // The track starts half a thumb (16 px at desktop density) inside the slider.
        input::click_at(
            viewing.slint(),
            LogicalPosition::new(x + 8.0 + 0.75 * (width - 16.0), y + height / 2.0),
        );
        wait_frame(
            &viewing,
            app,
            "the frame after a click at 75% of the slider",
            LATER,
            |state| state.position >= 0.7,
        )?;
        // The click gave the slider the keyboard: Home seeks to the start.
        input::key(viewing.slint(), Key::Home);
        wait_frame(
            &viewing,
            app,
            "the first frame again after Home",
            FIRST,
            |state| state.position <= 0.05,
        )?;
        input::click_button(viewing.slint(), "Play")?;
        wait_frame(
            &viewing,
            app,
            "Play to reach the later frames",
            LATER,
            |state| state.position >= 0.5,
        )?;
        viewing.wait(app, "the end of playback", STEP_LIMIT, |window| {
            let state = window.get_media_controls();
            !state.error.is_empty() || (!state.playing && state.position >= 0.99)
        })?;
        media_error(&viewing)
    })();
    let closed = viewing.close(app);
    result.and(closed)
}

/// The media bar's error, as the viewer shows it.
fn media_error(viewing: &Viewing) -> Result<(), String> {
    let state = viewing.window.get_media_controls();
    if state.error.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the viewer reports \"{}\" (status line \"{}\")",
            state.error,
            viewing.status()
        ))
    }
}

/// Wait until `ready` holds and the canvas shows `colour` over at least a sixteenth of
/// its area (the fitted frame fills most of it, the letterbox the rest); a media error
/// ends the wait with the viewer's sentence.
fn wait_frame(
    viewing: &Viewing,
    app: &mut App,
    what: &str,
    colour: [u8; 3],
    ready: impl Fn(&midnight_viewer_ui::MediaControlsState) -> bool,
) -> Result<(), String> {
    let waited = viewing.wait(app, what, STEP_LIMIT, |window| {
        let state = window.get_media_controls();
        !state.error.is_empty() || ready(&state)
    });
    media_error(viewing)?;
    waited?;
    let started = Instant::now();
    loop {
        let canvas = viewing.canvas();
        let shown = canvas.count_near(colour, TOLERANCE);
        if shown * 16 >= canvas.width * canvas.height {
            return Ok(());
        }
        if started.elapsed() >= STEP_LIMIT {
            return Err(format!(
                "{what}: the canvas shows {shown} of {} pixels in {colour:?}; media status \
                 \"{}\"",
                canvas.width * canvas.height,
                viewing.window.get_media_controls().status
            ));
        }
        app.idle(Duration::from_millis(20));
        support::paint_tool_windows(app);
        media_error(viewing)?;
    }
}

/// An MP4 file with twenty uncompressed 64 x 48 RGB frames at ten per second: ten in
/// [`FIRST`], then ten in [`LATER`]. One chunk per frame; every frame is a key frame.
fn raw_mp4() -> Vec<u8> {
    fn container(kind: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
        let length: usize = 8 + parts.iter().map(|part| part.len()).sum::<usize>();
        let mut bytes = Vec::with_capacity(length);
        bytes.extend((length as u32).to_be_bytes());
        bytes.extend(kind);
        for part in parts {
            bytes.extend(*part);
        }
        bytes
    }
    fn full(kind: &[u8; 4], flags: u32, body: &[u8]) -> Vec<u8> {
        container(kind, &[&flags.to_be_bytes(), body])
    }
    let frame_bytes = usize::from(WIDTH) * usize::from(HEIGHT) * 3;
    let frames: Vec<[u8; 3]> = (0..20)
        .map(|index| if index < 10 { FIRST } else { LATER })
        .collect();
    let duration = frames.len() as u32;
    let ftyp = container(
        b"ftyp",
        &[b"isom", &0x200u32.to_be_bytes(), b"isomiso2mp41"],
    );
    let mut media = Vec::with_capacity(frames.len() * frame_bytes);
    for colour in &frames {
        for _ in 0..usize::from(WIDTH) * usize::from(HEIGHT) {
            media.extend(colour);
        }
    }
    let mdat = container(b"mdat", &[&media]);
    let first_frame = (ftyp.len() + 8) as u32;
    let identity: Vec<u8> = [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect();
    let mut mvhd = Vec::new();
    for value in [0u32, 0, RATE, duration, 0x0001_0000] {
        mvhd.extend(value.to_be_bytes()); // times, timescale, duration, rate
    }
    mvhd.extend(0x0100u16.to_be_bytes()); // volume
    mvhd.extend([0; 10]);
    mvhd.extend(&identity);
    mvhd.extend([0; 24]);
    mvhd.extend(2u32.to_be_bytes()); // next track
    let mut tkhd = Vec::new();
    for value in [0u32, 0, 1, 0, duration, 0, 0] {
        tkhd.extend(value.to_be_bytes()); // times, track 1, duration
    }
    tkhd.extend([0; 8]); // layer, alternate group, volume, reserved
    tkhd.extend(&identity);
    tkhd.extend((u32::from(WIDTH) << 16).to_be_bytes());
    tkhd.extend((u32::from(HEIGHT) << 16).to_be_bytes());
    let mut mdhd = Vec::new();
    for value in [0u32, 0, RATE, duration] {
        mdhd.extend(value.to_be_bytes());
    }
    mdhd.extend(0x55C4u16.to_be_bytes()); // "und"
    mdhd.extend([0; 2]);
    let mut hdlr = vec![0; 4];
    hdlr.extend(b"vide");
    hdlr.extend([0; 12]);
    hdlr.extend(b"Video\0");
    let mut sample = vec![0; 6];
    sample.extend(1u16.to_be_bytes()); // data reference
    sample.extend([0; 16]);
    sample.extend(WIDTH.to_be_bytes());
    sample.extend(HEIGHT.to_be_bytes());
    sample.extend(0x0048_0000u32.to_be_bytes());
    sample.extend(0x0048_0000u32.to_be_bytes());
    sample.extend([0; 4]);
    sample.extend(1u16.to_be_bytes()); // frames per sample
    sample.extend([0; 32]); // compressor name
    sample.extend(24u16.to_be_bytes()); // depth: RGB
    sample.extend((-1i16).to_be_bytes()); // no colour table
    let entry = container(b"raw ", &[&sample]);
    let stsd = full(b"stsd", 0, &[&1u32.to_be_bytes()[..], &entry[..]].concat());
    let stts = full(
        b"stts",
        0,
        &[1u32, duration, 1].map(u32::to_be_bytes).concat(),
    );
    let stsc = full(b"stsc", 0, &[1u32, 1, 1, 1].map(u32::to_be_bytes).concat());
    let stsz = full(
        b"stsz",
        0,
        &[frame_bytes as u32, duration]
            .map(u32::to_be_bytes)
            .concat(),
    );
    let mut offsets = duration.to_be_bytes().to_vec();
    for index in 0..duration {
        offsets.extend((first_frame + index * frame_bytes as u32).to_be_bytes());
    }
    let stco = full(b"stco", 0, &offsets);
    let stbl = container(b"stbl", &[&stsd, &stts, &stsc, &stsz, &stco]);
    let location = full(b"url ", 1, &[]);
    let dref = full(
        b"dref",
        0,
        &[&1u32.to_be_bytes()[..], &location[..]].concat(),
    );
    let minf = container(
        b"minf",
        &[
            &full(b"vmhd", 1, &[0; 8]),
            &container(b"dinf", &[&dref]),
            &stbl,
        ],
    );
    let mdia = container(
        b"mdia",
        &[&full(b"mdhd", 0, &mdhd), &full(b"hdlr", 0, &hdlr), &minf],
    );
    let trak = container(b"trak", &[&full(b"tkhd", 3, &tkhd), &mdia]);
    let moov = container(b"moov", &[&full(b"mvhd", 0, &mvhd), &trak]);
    [ftyp, mdat, moov].concat()
}

/// The H.264 movie and its copy for compression, written by Windows' own encoders.
#[cfg(windows)]
fn h264(folder: &Path) -> Result<(), String> {
    h264::write(&folder.join(H264), FIRST, LATER)?;
    std::fs::copy(folder.join(H264), folder.join(H264_PACKED))
        .map(|_| ())
        .map_err(|error| format!("{}: {error}", folder.join(H264_PACKED).display()))
}

/// The H.264 + AAC movie, made by the host's `ffmpeg` with `libx264` and FFmpeg's own AAC
/// encoder: two seconds of 320 x 240 at ten frames per second, ten frames of [`FIRST`]
/// and ten of [`LATER`] (a key frame each), with silent stereo sound. Err with the
/// reason when this machine has no `ffmpeg` that can.
#[cfg(not(windows))]
fn h264(folder: &Path) -> Result<(), String> {
    let colour = |[red, green, blue]: [u8; 3]| {
        format!("color=c=0x{red:02X}{green:02X}{blue:02X}:s=320x240:r=10:d=1")
    };
    let (first, later) = (colour(FIRST), colour(LATER));
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            first.as_str(),
            "-f",
            "lavfi",
            "-i",
            later.as_str(),
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-map",
            "2:a",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "10",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
        ])
        .arg(folder.join(H264))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("no ffmpeg on this machine to make it ({error})"))?;
    if output.status.success() && folder.join(H264).is_file() {
        Ok(())
    } else {
        Err(format!(
            "the host's ffmpeg could not make it with libx264 ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// NTFS-compress one file, as Explorer's "Compress contents" or `compact /c` does.
#[cfg(windows)]
fn compress(path: &Path) -> Result<(), String> {
    compression::compress(path)
}

#[cfg(not(windows))]
fn compress(_: &Path) -> Result<(), String> {
    Err("NTFS compression is a Windows file attribute".into())
}

#[cfg(windows)]
mod compression {
    //! `FSCTL_SET_COMPRESSION` on a file handle: no shell or command text sees the path.
    #![allow(
        unsafe_code,
        reason = "one audited DeviceIoControl call, like the junction fixture"
    )]
    use std::{
        ffi::c_void,
        fs::OpenOptions,
        os::windows::{fs::MetadataExt, io::AsRawHandle},
        path::Path,
        ptr,
    };

    const FSCTL_SET_COMPRESSION: u32 = 0x0009_C040;
    const COMPRESSION_FORMAT_DEFAULT: u16 = 1;
    const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            device: *mut c_void,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }

    pub(super) fn compress(path: &Path) -> Result<(), String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut returned = 0;
        // SAFETY: an open file handle owned for the call; a two-byte input and no output.
        let done = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_COMPRESSION,
                (&COMPRESSION_FORMAT_DEFAULT as *const u16).cast(),
                size_of::<u16>() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if done == 0 {
            return Err(format!(
                "{} cannot be compressed here: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        drop(file);
        let attributes = std::fs::metadata(path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .file_attributes();
        if attributes & FILE_ATTRIBUTE_COMPRESSED == 0 {
            return Err(format!(
                "{} is not compressed after FSCTL_SET_COMPRESSION",
                path.display()
            ));
        }
        Ok(())
    }
}
