//! Installed first-party playback provider and its shared decoder runtime.

use midnight_plugin_host::{CapturedFile, TrustedExecutablePin};
use serde::Deserialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const VIDEO_PROVIDER_ID: &str = "midnight.viewer.video";
pub const VIDEO_RUNTIME_PROFILE: &str = "ffmpeg-9.0.2-playback-r4";

const DESCRIPTOR: &str = "midnight-thumbnail-video.json";
const MAX_DESCRIPTOR_BYTES: u64 = 16 * 1024;
#[cfg(windows)]
const WORKER_EXECUTABLE: &str = "midnight-video-playback.exe";
#[cfg(not(windows))]
const WORKER_EXECUTABLE: &str = "midnight-video-playback";
#[cfg(windows)]
const THUMBNAIL_EXECUTABLE: &str = "midnight-thumbnail-video.exe";
#[cfg(not(windows))]
const THUMBNAIL_EXECUTABLE: &str = "midnight-thumbnail-video";
/// The platform's decoders (Media Foundation on Windows; the distribution's FFmpeg, then
/// GStreamer, on Linux) in a helper that loads no bundled FFmpeg.
#[cfg(windows)]
const PLATFORM_WORKER_EXECUTABLE: &str = "midnight-video-playback-system.exe";
#[cfg(not(windows))]
const PLATFORM_WORKER_EXECUTABLE: &str = "midnight-video-playback-system";

#[cfg(windows)]
const LIBRARIES: &[&str] = &[
    "avcodec-63.dll",
    "avformat-63.dll",
    "avutil-61.dll",
    "swscale-10.dll",
    "swresample-7.dll",
];
#[cfg(not(windows))]
const LIBRARIES: &[&str] = &[
    "libavcodec.so.63",
    "libavformat.so.63",
    "libavutil.so.61",
    "libswscale.so.10",
    "libswresample.so.7",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    protocol: u32,
    id: String,
    version: String,
    executable: String,
    extensions: Vec<String>,
    video: bool,
}

/// Which installed playback helper decodes a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoWorker {
    /// The bundled FFmpeg profile.
    Bundled,
    /// The platform's decoders.
    Platform,
}

/// Containers that usually hold video the bundled profile has no decoder for (H.264, HEVC,
/// Windows Media) and the system's decoders play: their first helper is the platform's,
/// so an opening does not start, probe and retire a bundled helper first.
#[cfg(windows)]
const PLATFORM_FIRST: &[&str] = &["mp4", "m4v", "mov", "3gp", "asf", "wmv", "avi"];
#[cfg(not(windows))]
const PLATFORM_FIRST: &[&str] = &[];

/// Whether the platform's helper should open `name` first (by its suffix alone: the host
/// never parses the video).
fn platform_first(name: &Path) -> bool {
    name.extension()
        .and_then(|suffix| suffix.to_str())
        .is_some_and(|suffix| {
            PLATFORM_FIRST
                .iter()
                .any(|known| known.eq_ignore_ascii_case(suffix))
        })
}

/// The helper to try when `first` could not open a video with `error`, if any. The bundled
/// helper hands over only a video it has no decoder for; the platform's helper hands over
/// whatever it could not open (a container or codec its decoders do not read, or decoders
/// that are not installed), unless the opening was cancelled.
pub fn fallback_worker(
    first: VideoWorker,
    error: &io::Error,
    platform_installed: bool,
) -> Option<VideoWorker> {
    match first {
        VideoWorker::Bundled => (platform_installed
            && crate::system_decoders::MissingDecoder::find(error).is_some())
        .then_some(VideoWorker::Platform),
        VideoWorker::Platform => {
            (error.kind() != io::ErrorKind::Interrupted).then_some(VideoWorker::Bundled)
        }
    }
}

/// The error to report when both helpers failed: the decoders both lacked, or the error of
/// the helper whose failure says more. A platform helper that had a decoder but failed
/// explains more than the bundled profile's missing decoder.
pub fn both_failed(
    first: VideoWorker,
    first_error: io::Error,
    second_error: io::Error,
) -> io::Error {
    use crate::system_decoders::MissingDecoder;
    let (bundled, platform) = match first {
        VideoWorker::Bundled => (first_error, second_error),
        VideoWorker::Platform => (second_error, first_error),
    };
    match (
        MissingDecoder::find(&bundled),
        MissingDecoder::find(&platform),
    ) {
        (Some(bundled), Some(platform)) => bundled.then(platform).into(),
        (Some(_), None) => platform,
        _ => bundled,
    }
}

#[derive(Clone, Debug)]
pub struct VideoInstallation {
    package: PathBuf,
    manifest: Manifest,
    /// The descriptor's bytes when it was discovered: each launch reads it again and
    /// compares the content. Not its native revision: NTFS moves a file's ChangeTime for
    /// changes that leave content alone (a hard link made or removed, an attribute or a
    /// security scan's stream), and a stale revision then refused every later video until
    /// the application restarted. The helpers' code is pinned on its own.
    descriptor_revision: Vec<u8>,
    /// Whether the platform helper is installed beside the bundled helpers.
    platform: bool,
}

impl VideoInstallation {
    /// Discover using the trusted application executable path supplied at startup.
    /// Call on a backend worker. The viewer catalog uses the descriptor's defaults.
    pub fn discover(app_executable: &Path) -> io::Result<Self> {
        if !app_executable.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video installation needs the absolute application executable",
            ));
        }
        let executable = midnight_plugin_host::installation_path(app_executable)?;
        let package = executable
            .parent()
            .ok_or_else(|| io::Error::other("Application executable has no parent"))?
            .join("thumbnailers");
        let (manifest, descriptor_revision) = inspect(&package)?;
        let platform = regular(&package.join(PLATFORM_WORKER_EXECUTABLE)).is_ok();
        Ok(Self {
            package,
            manifest,
            descriptor_revision,
            platform,
        })
    }

    pub fn extensions(&self) -> &[String] {
        &self.manifest.extensions
    }

    /// The helper that opens `name` first; [`fallback_worker`] names the other.
    pub fn first_worker(&self, name: &Path) -> VideoWorker {
        if self.platform && platform_first(name) {
            VideoWorker::Platform
        } else {
            VideoWorker::Bundled
        }
    }

    /// The platform helper, when it is installed.
    pub fn platform_worker(&self) -> Option<PathBuf> {
        self.platform
            .then(|| self.package.join(PLATFORM_WORKER_EXECUTABLE))
    }

    pub(super) fn worker(&self, which: VideoWorker) -> io::Result<PathBuf> {
        match which {
            VideoWorker::Bundled => Ok(self.package.join(WORKER_EXECUTABLE)),
            VideoWorker::Platform => self.platform_worker().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "The platform video helper is not installed",
                )
            }),
        }
    }

    /// The shared FFmpeg libraries that the bundled helpers' loader opens beside them. The
    /// platform helper loads none from its folder.
    pub(super) fn libraries(&self, which: VideoWorker) -> Vec<PathBuf> {
        match which {
            VideoWorker::Bundled => LIBRARIES
                .iter()
                .map(|name| self.package.join(name))
                .collect(),
            VideoWorker::Platform => Vec::new(),
        }
    }

    pub(super) fn capture(&self) -> io::Result<LaunchPins> {
        let mut owner = LaunchPins {
            installation: self.clone(),
            pins: Vec::with_capacity(3 + LIBRARIES.len()),
        };
        let platform = self.platform.then_some(PLATFORM_WORKER_EXECUTABLE);
        for file in program_names().chain(platform) {
            match TrustedExecutablePin::capture(&self.package, &self.package.join(file)) {
                Ok(pin) => owner.pins.push(pin),
                Err(error) => {
                    owner.release()?;
                    return Err(io::Error::other(error));
                }
            }
        }
        if let Err(error) = owner.revalidate() {
            owner.release()?;
            return Err(error);
        }
        Ok(owner)
    }
}

/// The host holds these code identities across spawn, Ready, decoding and verified exit.
/// A pin authenticates installed code paths. The pinned profile is qualified by Build separately.
pub(super) struct LaunchPins {
    installation: VideoInstallation,
    pins: Vec<TrustedExecutablePin>,
}

impl LaunchPins {
    pub(super) fn revalidate(&self) -> io::Result<()> {
        let (manifest, revision) = inspect(&self.installation.package)?;
        if manifest != self.installation.manifest
            || revision != self.installation.descriptor_revision
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Installed video descriptor changed",
            ));
        }
        for pin in &self.pins {
            pin.revalidate().map_err(io::Error::other)?;
        }
        Ok(())
    }

    /// Consume only after the process owner has confirmed native termination.
    pub(super) fn release(self) -> io::Result<()> {
        let mut failure = None;
        for pin in self.pins {
            if let Err(error) = pin.release_on_worker() {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), |error| Err(io::Error::other(error)))
    }
}

fn program_names() -> impl Iterator<Item = &'static str> {
    [WORKER_EXECUTABLE, THUMBNAIL_EXECUTABLE]
        .into_iter()
        .chain(LIBRARIES.iter().copied())
}

fn inspect(package: &Path) -> io::Result<(Manifest, Vec<u8>)> {
    let metadata = fs::symlink_metadata(package)?;
    if !metadata.file_type().is_dir() {
        return Err(invalid_descriptor());
    }
    midnight_plugin_host::validate_plugin_package(package).map_err(io::Error::other)?;
    for file in program_names() {
        regular(&package.join(file))?;
    }
    let descriptor = package.join(DESCRIPTOR);
    regular(&descriptor)?;
    let captured = CapturedFile::capture(&descriptor).map_err(io::Error::other)?;
    let result = read_descriptor(&captured);
    let release = captured.release_on_worker().map_err(io::Error::other);
    let descriptor = result?;
    release?;
    Ok(descriptor)
}

fn regular(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(invalid_descriptor());
    }
    midnight_plugin_host::validate_plugin_path(path).map_err(io::Error::other)
}

fn read_descriptor(capture: &CapturedFile) -> io::Result<(Manifest, Vec<u8>)> {
    let length = capture.length();
    if length == 0 || length > MAX_DESCRIPTOR_BYTES {
        return Err(invalid_descriptor());
    }
    let content = capture
        .read_at(0, length as u32)
        .map_err(io::Error::other)?;
    if content.len() as u64 != length {
        return Err(invalid_descriptor());
    }
    let manifest = parse_manifest(&content)?;
    Ok((manifest, content))
}

fn parse_manifest(content: &[u8]) -> io::Result<Manifest> {
    let mut manifest: Manifest =
        serde_json::from_slice(content).map_err(|_| invalid_descriptor())?;
    if manifest.protocol != 1
        || manifest.id != "midnight-video"
        || manifest.version != VIDEO_RUNTIME_PROFILE
        || manifest.executable != THUMBNAIL_EXECUTABLE
        || !manifest.video
        || !(1..=32).contains(&manifest.extensions.len())
    {
        return Err(invalid_descriptor());
    }
    for suffix in &mut manifest.extensions {
        if !(1..=16).contains(&suffix.len())
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err(invalid_descriptor());
        }
        suffix.make_ascii_lowercase();
    }
    for (index, suffix) in manifest.extensions.iter().enumerate() {
        if manifest.extensions[..index].contains(suffix) {
            return Err(invalid_descriptor());
        }
    }
    Ok(manifest)
}

fn invalid_descriptor() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Installed video provider does not match the playback profile",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> serde_json::Value {
        serde_json::json!({
            "protocol": 1,
            "id": "midnight-video",
            "version": VIDEO_RUNTIME_PROFILE,
            "executable": THUMBNAIL_EXECUTABLE,
            "extensions": ["WebM", "mp4"],
            "video": true,
        })
    }

    fn parse(value: &serde_json::Value) -> io::Result<Manifest> {
        parse_manifest(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn supported_descriptor_uses_the_same_extensions_as_the_thumbnail_runtime() {
        assert_eq!(parse(&descriptor()).unwrap().extensions, ["webm", "mp4"]);
    }

    #[test]
    fn unexpected_provider_version_executable_or_suffix_is_refused() {
        for (field, value) in [
            ("protocol", serde_json::json!(2)),
            ("id", serde_json::json!("foreign-video")),
            ("version", serde_json::json!("ffmpeg-9.0.2")),
            ("executable", serde_json::json!("../replacement.exe")),
            ("video", serde_json::json!(false)),
            ("extensions", serde_json::json!([".mp4"])),
            ("extensions", serde_json::json!(["mp4", "MP4"])),
            ("extensions", serde_json::json!([])),
            ("extra", serde_json::json!(true)),
        ] {
            let mut candidate = descriptor();
            candidate[field] = value;
            assert_eq!(
                parse(&candidate).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn the_container_chooses_the_first_helper_and_the_other_is_the_fallback() {
        use crate::system_decoders::{MissingDecoder, TRIED_BUNDLED, TRIED_MEDIA_FOUNDATION};
        for name in [
            "clip.mp4", "CLIP.MOV", "a.m4v", "a.3gp", "a.asf", "a.wmv", "a.avi",
        ] {
            assert_eq!(platform_first(Path::new(name)), cfg!(windows), "{name}");
        }
        for name in ["clip.webm", "clip.mkv", "clip.ogv", "mp4", "clip.mp4.part"] {
            assert!(!platform_first(Path::new(name)), "{name}");
        }

        let missing = |tried| io::Error::from(MissingDecoder { codec: 1, tried });
        let damaged = || io::Error::new(io::ErrorKind::InvalidData, "damaged");
        let cancelled = || io::Error::from(io::ErrorKind::Interrupted);
        // The bundled helper hands over only a missing decoder, and only to an installed
        // platform helper.
        assert_eq!(
            fallback_worker(VideoWorker::Bundled, &missing(TRIED_BUNDLED), true),
            Some(VideoWorker::Platform)
        );
        assert_eq!(
            fallback_worker(VideoWorker::Bundled, &missing(TRIED_BUNDLED), false),
            None
        );
        assert_eq!(
            fallback_worker(VideoWorker::Bundled, &damaged(), true),
            None
        );
        // The platform helper hands over whatever it could not open, unless cancelled.
        for error in [missing(TRIED_MEDIA_FOUNDATION), damaged()] {
            assert_eq!(
                fallback_worker(VideoWorker::Platform, &error, true),
                Some(VideoWorker::Bundled)
            );
        }
        assert_eq!(
            fallback_worker(VideoWorker::Platform, &cancelled(), true),
            None
        );

        // Both lacked a decoder: the report names everything that was tried.
        for first in [VideoWorker::Bundled, VideoWorker::Platform] {
            let (one, two) = match first {
                VideoWorker::Bundled => (missing(TRIED_BUNDLED), missing(TRIED_MEDIA_FOUNDATION)),
                VideoWorker::Platform => (missing(TRIED_MEDIA_FOUNDATION), missing(TRIED_BUNDLED)),
            };
            assert_eq!(
                MissingDecoder::find(&both_failed(first, one, two)),
                Some(MissingDecoder {
                    codec: 1,
                    tried: TRIED_BUNDLED | TRIED_MEDIA_FOUNDATION
                })
            );
        }
        // A platform decoder that failed says more than the bundled profile's missing one,
        // and a bundled failure that is not a missing decoder is the bundled helper's.
        let report = both_failed(VideoWorker::Platform, damaged(), missing(TRIED_BUNDLED));
        assert_eq!(report.kind(), io::ErrorKind::InvalidData);
        let report = both_failed(
            VideoWorker::Platform,
            missing(TRIED_MEDIA_FOUNDATION),
            damaged(),
        );
        assert_eq!(report.kind(), io::ErrorKind::InvalidData);
        assert!(MissingDecoder::find(&report).is_none());
    }
}
