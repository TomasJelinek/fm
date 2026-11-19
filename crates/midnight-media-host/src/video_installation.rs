//! Installed first-party playback provider and its shared decoder runtime.

use midnight_plugin_host::{CapturedFile, TrustedExecutablePin};
use serde::Deserialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const VIDEO_PROVIDER_ID: &str = "midnight.viewer.video";
pub const VIDEO_RUNTIME_PROFILE: &str = "ffmpeg-9.0.2-playback-r3";

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

#[derive(Clone, Debug)]
pub struct VideoInstallation {
    package: PathBuf,
    manifest: Manifest,
    descriptor_revision: Vec<u8>,
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
        Ok(Self {
            package,
            manifest,
            descriptor_revision,
        })
    }

    pub fn extensions(&self) -> &[String] {
        &self.manifest.extensions
    }

    pub(super) fn worker(&self) -> PathBuf {
        self.package.join(WORKER_EXECUTABLE)
    }

    pub(super) fn capture(&self) -> io::Result<LaunchPins> {
        let mut owner = LaunchPins {
            installation: self.clone(),
            pins: Vec::with_capacity(2 + LIBRARIES.len()),
        };
        for file in program_names() {
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
    Ok((parse_manifest(&content)?, capture.stable_version()))
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
}
