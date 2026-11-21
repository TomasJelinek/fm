//! Media Foundation decoding of H.264 + AAC MP4 movies that Media Foundation's own encoders write
//! at test time. Windows only: the platform helpers of other systems decode differently.
#![cfg(windows)]

#[path = "fixtures/mf_writer.rs"]
mod writer;

use midnight_thumbnail_sdk::{ReadSeek, api::Request};
use midnight_thumbnail_video::{
    media::{AudioTrack, Codec, NoDecoder, OutputBounds, Tried},
    playback_worker::{PlaybackDecoder, Sources},
    system::media_foundation,
};
use std::{io, sync::OnceLock};
use writer::{FIRST, LATER, Movie};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

/// One movie for the whole test binary: encoding takes a moment.
fn movie() -> &'static [u8] {
    static MOVIE: OnceLock<Vec<u8>> = OnceLock::new();
    MOVIE.get_or_init(|| {
        Movie::new(WIDTH, HEIGHT)
            .write()
            .expect("Media Foundation encodes the fixture")
    })
}

struct Memory<'a>(&'a [u8]);

impl Sources for Memory<'_> {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }

    fn cursor(&self) -> Box<dyn ReadSeek + '_> {
        Box::new(io::Cursor::new(self.0))
    }
}

fn thumbnail(
    bytes: &[u8],
    frame: u8,
    width: u32,
    height: u32,
) -> io::Result<midnight_thumbnail_sdk::api::Frame> {
    let request = Request {
        id: 7,
        source_length: bytes.len() as u64,
        width,
        height,
        frame,
    };
    media_foundation::thumbnail(&request, &mut io::Cursor::new(bytes))
}

/// A decoded pixel shows `color` (after the BT.709-to-sRGB transfer both decoders apply).
fn shows(pixel: &[u8], color: [u8; 3]) -> bool {
    let high = |value: u8| value > 140;
    let low = |value: u8| value < 110;
    match color {
        FIRST => high(pixel[0]) && low(pixel[1]) && high(pixel[2]),
        LATER => low(pixel[0]) && high(pixel[1]) && low(pixel[2]),
        _ => unreachable!("fixture colors only"),
    }
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> &[u8] {
    let start = ((y * width + x) * 4) as usize;
    &rgba[start..start + 4]
}

#[test]
fn an_h264_thumbnail_shows_the_picture_at_its_sample_position() {
    let bytes = movie();
    // 10% of the duration is in the first half, 85% in the second, where the halves swap.
    for (frame, left, right) in [(0, FIRST, LATER), (5, LATER, FIRST)] {
        let thumbnail = thumbnail(bytes, frame, 64, 64).unwrap();
        assert_eq!((thumbnail.id, thumbnail.frame), (7, frame));
        assert_eq!((thumbnail.width, thumbnail.height), (64, 36));
        assert_eq!(thumbnail.rgba.len(), 64 * 36 * 4);
        let (w, h) = (thumbnail.width, thumbnail.height);
        assert!(
            shows(pixel(&thumbnail.rgba, w, w / 4, h / 2), left),
            "sample {frame} left"
        );
        assert!(
            shows(pixel(&thumbnail.rgba, w, w * 3 / 4, h / 2), right),
            "sample {frame} right"
        );
        assert!(thumbnail.rgba.chunks(4).all(|pixel| pixel[3] == 255));
    }
}

#[test]
fn playback_starts_with_the_first_frame_seeks_and_plays_its_audio() {
    let bytes = movie();
    let bounds = OutputBounds {
        width: 160,
        height: 160,
    };
    let mut checked = false;
    media_foundation::open(
        &Memory(bytes),
        bounds,
        &mut |decoder: &mut dyn PlaybackDecoder| {
            let metadata = decoder.metadata();
            assert_eq!((metadata.width, metadata.height), (WIDTH, HEIGHT));
            let duration = metadata
                .duration_micros
                .expect("an MP4 states its duration");
            assert!((1_900_000..=2_200_000).contains(&duration), "{duration}");
            assert_eq!(decoder.audio_track(), AudioTrack::Qualified);

            let first = decoder.next_frame()?.expect("the first frame");
            assert_eq!(first.presentation_micros, 0);
            assert_eq!((first.width, first.height), (160, 90));
            assert!(shows(pixel(&first.rgba, 160, 40, 45), FIRST));
            assert!(shows(pixel(&first.rgba, 160, 120, 45), LATER));
            let second = decoder.next_frame()?.expect("the second frame");
            assert_eq!(second.presentation_micros, 100_000);

            decoder.seek(1_500_000)?;
            let later = decoder.next_frame()?.expect("a frame after the seek");
            assert_eq!(later.presentation_micros, 1_500_000);
            assert!(shows(pixel(&later.rgba, 160, 40, 45), LATER));

            decoder.seek(0)?;
            let again = decoder.next_frame()?.expect("the first frame again");
            assert_eq!(again.presentation_micros, 0);
            assert!(shows(pixel(&again.rgba, 160, 40, 45), FIRST));

            let block = decoder.pcm(4_800)?.expect("audio after the seek");
            assert_eq!(block.first_frame, 0);
            assert_eq!(block.samples.len(), 4_800 * 2);
            // Skip the encoder's priming; the 440 Hz tone has a clear level after it.
            let tone = &block.samples[2_400..];
            let level = tone
                .iter()
                .map(|sample| i64::from(*sample).abs())
                .sum::<i64>()
                / tone.len() as i64;
            assert!(level > 1_000, "audio level {level}");
            let next = decoder.pcm(4_800)?.expect("contiguous audio");
            assert_eq!(next.first_frame, 4_800);

            // Frame 0 was read after seeking back; the rest of the movie follows it.
            let mut frames = 1;
            while decoder.next_frame()?.is_some() {
                frames += 1;
            }
            assert_eq!(frames, 20);
            assert!(
                decoder.next_frame()?.is_none(),
                "the end stays until a seek"
            );
            checked = true;
            Ok(())
        },
    )
    .unwrap();
    assert!(checked);
}

#[test]
fn audio_at_another_rate_arrives_as_48_khz_stereo() {
    let mut cd_rate = Movie::new(WIDTH, HEIGHT);
    cd_rate.audio_rate = 44_100;
    let bytes = cd_rate.write().unwrap();
    let bounds = OutputBounds {
        width: 64,
        height: 64,
    };
    media_foundation::open(
        &Memory(&bytes),
        bounds,
        &mut |decoder: &mut dyn PlaybackDecoder| {
            assert_eq!(decoder.audio_track(), AudioTrack::Qualified);
            // Two seconds at 48 kHz, whatever the source rate.
            let mut frames = 0;
            let mut level = 0i64;
            while let Some(block) = decoder.pcm(4_800)? {
                assert_eq!(block.first_frame, frames);
                frames += block.samples.len() as u64 / 2;
                level += block
                    .samples
                    .iter()
                    .map(|sample| i64::from(*sample).abs())
                    .sum::<i64>();
            }
            assert!((95_000..=97_000).contains(&frames), "{frames} frames");
            assert!(
                level / frames as i64 > 1_000,
                "audio level {}",
                level / frames as i64
            );
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn a_movie_without_audio_plays_its_video_alone() {
    let mut silent = Movie::new(WIDTH, HEIGHT);
    silent.audio = false;
    let bytes = silent.write().unwrap();
    let bounds = OutputBounds {
        width: 64,
        height: 64,
    };
    media_foundation::open(
        &Memory(&bytes),
        bounds,
        &mut |decoder: &mut dyn PlaybackDecoder| {
            assert_eq!(decoder.audio_track(), AudioTrack::Absent);
            assert!(decoder.next_frame()?.is_some());
            assert!(decoder.pcm(4_800)?.is_none());
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn a_movie_recorded_turned_is_shown_upright() {
    let mut bytes = movie().to_vec();
    assert!(
        writer::rotate(&mut bytes, 1),
        "the fixture has a video track header"
    );
    let thumbnail = thumbnail(&bytes, 0, 64, 64).unwrap();
    // Turned a quarter clockwise, the picture stands: the left half is now on top.
    assert_eq!((thumbnail.width, thumbnail.height), (36, 64));
    let (w, h) = (thumbnail.width, thumbnail.height);
    assert!(shows(pixel(&thumbnail.rgba, w, w / 2, h / 4), FIRST));
    assert!(shows(pixel(&thumbnail.rgba, w, w / 2, h * 3 / 4), LATER));
    media_foundation::open(
        &Memory(&bytes),
        OutputBounds {
            width: 64,
            height: 64,
        },
        &mut |decoder: &mut dyn PlaybackDecoder| {
            let metadata = decoder.metadata();
            assert_eq!((metadata.width, metadata.height), (HEIGHT, WIDTH));
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn a_codec_without_a_windows_decoder_is_named() {
    let mut bytes = movie().to_vec();
    // Apple ProRes 422: Windows has no decoder for it.
    assert!(writer::replace_sample_entry(&mut bytes, b"avc1", b"apcn"));
    let error = thumbnail(&bytes, 0, 64, 64).unwrap_err();
    let missing =
        NoDecoder::find(&error).unwrap_or_else(|| panic!("not a missing decoder: {error}"));
    assert!(missing.tried.contains(Tried::MEDIA_FOUNDATION));
    assert!(
        matches!(missing.codec, Codec::ProRes | Codec::Unknown),
        "{missing:?}"
    );
    let error = media_foundation::open(
        &Memory(&bytes),
        OutputBounds {
            width: 64,
            height: 64,
        },
        &mut |_: &mut dyn PlaybackDecoder| panic!("no decoder opened"),
    )
    .unwrap_err();
    assert_eq!(NoDecoder::find(&error), Some(missing));
}

#[test]
fn bytes_that_are_no_movie_report_that_no_decoder_reads_them() {
    for bytes in [
        b"not a video".as_slice(),
        b"#EXTM3U\nhttp://127.0.0.1/private\n",
    ] {
        let error = thumbnail(bytes, 0, 32, 32).unwrap_err();
        assert_eq!(
            NoDecoder::find(&error),
            Some(NoDecoder {
                codec: Codec::Unknown,
                tried: Tried::MEDIA_FOUNDATION,
            }),
            "{error}"
        );
    }
}

/// An HEVC movie, when this system has an HEVC encoder (often a GPU's); `None` otherwise.
fn hevc() -> Option<&'static [u8]> {
    static MOVIE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    MOVIE
        .get_or_init(|| {
            let mut movie = Movie::new(WIDTH, HEIGHT);
            movie.video = windows::Win32::Media::MediaFoundation::MFVideoFormat_HEVC;
            movie.hardware = true;
            movie.audio = false;
            movie
                .write()
                .inspect_err(|error| println!("No HEVC fixture: {error}"))
                .ok()
        })
        .as_deref()
}

/// HEVC decodes through the HEVC Video Extensions (a Microsoft Store package) when they are
/// installed, and is otherwise named as the missing codec.
#[test]
fn hevc_decodes_with_its_store_extension_or_names_the_missing_codec() {
    let Some(bytes) = hevc() else {
        println!("SKIPPED: this system cannot encode HEVC, so no fixture exists");
        return;
    };
    match thumbnail(bytes, 0, 64, 64) {
        Ok(thumbnail) => {
            assert_eq!((thumbnail.width, thumbnail.height), (64, 36));
            assert!(shows(pixel(&thumbnail.rgba, 64, 16, 18), FIRST));
            println!("HEVC decoded through the installed extension");
        }
        Err(error) => {
            let missing = NoDecoder::find(&error).unwrap_or_else(|| panic!("{error}"));
            assert_eq!(missing.codec, Codec::Hevc);
            println!("HEVC has no decoder here: {error}");
        }
    }
}

/// The fixtures for the sandboxed helper test (midnight-thumbnail-host `tests/native_video.rs`):
/// `h264.mp4`, and `hevc.mp4` where this system encodes HEVC and decodes it unrestricted.
#[test]
#[ignore = "writes fixtures to MIDNIGHT_MF_FIXTURE_DIR for the sandboxed helper test"]
fn write_fixtures_for_the_sandboxed_helper_test() {
    let folder = std::path::PathBuf::from(
        std::env::var_os("MIDNIGHT_MF_FIXTURE_DIR")
            .expect("MIDNIGHT_MF_FIXTURE_DIR names a folder"),
    );
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("h264.mp4"), movie()).unwrap();
    match hevc() {
        Some(bytes) if thumbnail(bytes, 0, 64, 64).is_ok() => {
            std::fs::write(folder.join("hevc.mp4"), bytes).unwrap();
        }
        Some(_) => println!("HEVC does not decode here; no hevc.mp4"),
        None => println!("No HEVC encoder here; no hevc.mp4"),
    }
}

const SIGNED_ONLY_TEST: &str =
    "media_foundation_decodes_with_only_signed_images_and_loads_only_windows_code";
const SIGNED_ONLY_CHILD: &str = "MIDNIGHT_MF_SIGNED_ONLY_CHILD";

/// The helpers accept only Microsoft-signed images before decoding. Decoding must still work,
/// Microsoft's Store codec extensions included, and everything Media Foundation loads must come
/// from Windows or a Store package.
#[test]
fn media_foundation_decodes_with_only_signed_images_and_loads_only_windows_code() {
    if std::env::var_os(SIGNED_ONLY_CHILD).is_none() {
        // The policy cannot be undone, so it runs in a child copy of this test. The child
        // must decode HEVC exactly when this unrestricted process can.
        let hevc_decodes = hevc().is_some_and(|bytes| thumbnail(bytes, 0, 64, 64).is_ok());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                SIGNED_ONLY_TEST,
                "--nocapture",
                "--test-threads=1",
            ])
            .env(SIGNED_ONLY_CHILD, "1")
            .env(
                "MIDNIGHT_MF_HEVC_DECODES",
                if hevc_decodes { "1" } else { "0" },
            )
            .output()
            .unwrap();
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{:?}", output.status);
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let bytes = movie();
    let store_codec = hevc();
    let before = modules::loaded();
    media_foundation::restrict_images().unwrap();
    if let Some(store_codec) = store_codec {
        // A Store codec extension is admitted: HEVC decodes as it does without the policy
        // (the parent decoded it unrestricted; its decoder loads here for the first time).
        let decoded = thumbnail(store_codec, 0, 64, 64);
        println!(
            "HEVC with the policy: {:?}",
            decoded.as_ref().map(|frame| (frame.width, frame.height))
        );
        let expected = std::env::var("MIDNIGHT_MF_HEVC_DECODES").is_ok_and(|value| value == "1");
        let size = decoded.as_ref().map(|frame| (frame.width, frame.height));
        assert_eq!(decoded.is_ok(), expected, "{size:?}");
    }
    let thumbnail = thumbnail(bytes, 0, 64, 64).unwrap();
    assert!(shows(pixel(&thumbnail.rgba, 64, 16, 18), FIRST));
    media_foundation::open(
        &Memory(bytes),
        OutputBounds {
            width: 64,
            height: 64,
        },
        &mut |decoder: &mut dyn PlaybackDecoder| {
            assert!(decoder.next_frame()?.is_some());
            assert!(decoder.pcm(4_800)?.is_some());
            Ok(())
        },
    )
    .unwrap();
    let windows = std::env::var("SystemRoot").unwrap().to_lowercase();
    let store = std::env::var("ProgramFiles").unwrap().to_lowercase() + "\\windowsapps\\";
    let mut foreign = Vec::new();
    for module in modules::loaded().difference(&before) {
        println!("Media Foundation loaded {module}");
        let lower = module.to_lowercase();
        if !lower.starts_with(&windows) && !lower.starts_with(&store) {
            foreign.push(module.clone());
        }
    }
    assert!(
        foreign.is_empty(),
        "modules from outside Windows: {foreign:?}"
    );
    // An unsigned image, as a codec pack would install, cannot load any more: this test's own
    // executable, which no one signed, copied under a library's name.
    let copy = std::env::temp_dir().join(format!("midnight-unsigned-{}.dll", std::process::id()));
    std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
    let refused = modules::load(&copy);
    let _ = std::fs::remove_file(&copy);
    // ERROR_INVALID_IMAGE_HASH: the image does not meet the signing level.
    assert_eq!(refused, Err(577), "an unsigned library loaded");
}

mod modules {
    use std::{collections::BTreeSet, path::Path};
    use windows::{
        Win32::{
            Foundation::{FreeLibrary, HMODULE},
            System::{
                LibraryLoader::LoadLibraryW,
                ProcessStatus::{EnumProcessModules, GetModuleFileNameExW},
                Threading::GetCurrentProcess,
            },
        },
        core::HSTRING,
    };

    /// Load a library as code; the Win32 error when that is refused.
    pub fn load(path: &Path) -> Result<(), u32> {
        // SAFETY: a test library path; a loaded library is released at once.
        unsafe {
            match LoadLibraryW(&HSTRING::from(path)) {
                Ok(module) => {
                    let _ = FreeLibrary(module);
                    Ok(())
                }
                Err(error) => Err((error.code().0 as u32) & 0xFFFF),
            }
        }
    }

    /// The full paths of the modules loaded in this process.
    pub fn loaded() -> BTreeSet<String> {
        let mut handles = vec![HMODULE::default(); 1024];
        let mut needed = 0u32;
        // SAFETY: the buffer holds `handles.len()` module handles; names are read into owned
        // buffers of the size passed.
        unsafe {
            let process = GetCurrentProcess();
            EnumProcessModules(
                process,
                handles.as_mut_ptr(),
                (handles.len() * size_of::<HMODULE>()) as u32,
                &mut needed,
            )
            .unwrap();
            handles.truncate(needed as usize / size_of::<HMODULE>());
            handles
                .into_iter()
                .map(|module| {
                    let mut name = [0u16; 1024];
                    let length = GetModuleFileNameExW(Some(process), Some(module), &mut name);
                    String::from_utf16_lossy(&name[..length as usize])
                })
                .collect()
        }
    }
}
