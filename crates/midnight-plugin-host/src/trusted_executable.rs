//! Backend-owned identity for one trusted executable, separate from content captures.
use crate::Error;
use std::path::{Path, PathBuf};

#[cfg(windows)]
#[path = "trusted_executable_windows.rs"]
mod windows;
#[cfg(not(windows))]
use crate::CapturedFile as PlatformPin;
#[cfg(windows)]
use windows::ExecutableLease as PlatformPin;

/// An opaque, non-cloneable pin for trusted executable code.
///
/// Capture, validation, explicit release and fallback Drop belong on backend workers.
/// On Windows, this owns an uninterrupted read-data handle which denies data writes,
/// deletion and replacement. Only ChangeTime differences are tolerated under that lease;
/// file identity, length, LastWriteTime and current package/path trust still must match.
/// Other platforms retain the strict `CapturedFile` revision contract, including ctime.
/// This pin protects one executable; it does not validate unrelated package contents.
/// Callers must retain full provider/package validation before launch and adoption.
/// This type neither hashes executable contents nor weakens ordinary content captures.
///
/// The process/transport owner must keep the pin until native termination is confirmed.
/// If termination is unconfirmed, retain the entire owner instead of dropping this pin.
/// Its backend Drop fallback is appropriate only when retiring the pin is already safe.
pub struct TrustedExecutablePin {
    package: PathBuf,
    executable: PathBuf,
    pin: PlatformPin,
}

impl std::fmt::Debug for TrustedExecutablePin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrustedExecutablePin")
            .finish_non_exhaustive()
    }
}

impl TrustedExecutablePin {
    /// Capture a regular executable directly inside its current trusted canonical package.
    /// Windows obtains the read-data/write-deny lease before its initial native snapshot.
    pub fn capture(package: &Path, executable: &Path) -> Result<Self, Error> {
        validate_location(package, executable)?;
        let pin = PlatformPin::capture(executable)?;
        let owner = Self {
            package: package.to_owned(),
            executable: executable.to_owned(),
            pin,
        };
        if let Err(error) = owner.revalidate() {
            return match owner.release_on_worker() {
                Ok(()) => Err(error),
                Err(close) => Err(Error(format!(
                    "{error}; trusted executable pin cleanup is unconfirmed: {close}",
                ))),
            };
        }
        Ok(owner)
    }

    /// Recheck both held and currently named identity without releasing the Windows lease.
    /// Current package ownership, canonical paths and no-reparse restrictions also apply.
    /// An unconfirmed temporary probe close is sticky: later validation cannot open a new
    /// probe or succeed, and consuming release still reports that unknown cleanup tail.
    pub fn revalidate(&self) -> Result<(), Error> {
        #[cfg(windows)]
        self.pin.ensure_confirmed()?;
        validate_location(&self.package, &self.executable)?;
        #[cfg(windows)]
        self.pin.revalidate(&self.executable)?;
        #[cfg(not(windows))]
        self.pin.revalidate()?;
        validate_location(&self.package, &self.executable)?;
        #[cfg(windows)]
        self.pin.ensure_confirmed()?;
        Ok(())
    }

    /// Consume the pin on a backend worker after the native process is confirmed stopped.
    ///
    /// Windows closes its actual retained handle once. An error or unwind during explicit
    /// close retains the unconfirmed native owner; Drop never attempts a second close after
    /// that uncertain outcome. The caller must propagate unconfirmed cleanup rather than
    /// treating this result as confirmed retirement. Any earlier unconfirmed probe remains
    /// an error even when the held handle closes successfully. No retry or hashing occurs.
    pub fn release_on_worker(self) -> Result<(), Error> {
        self.pin.release_on_worker()
    }
}

fn validate_location(package: &Path, executable: &Path) -> Result<(), Error> {
    if !package.is_absolute() || executable.parent() != Some(package) {
        return Err(Error(
            "Executable must be directly inside its absolute canonical package".into(),
        ));
    }
    let kind = std::fs::symlink_metadata(package)?.file_type();
    if !kind.is_dir() || kind.is_symlink() {
        return Err(Error(
            "Executable package must be a non-symlink directory".into(),
        ));
    }
    crate::validate_plugin_path(package)?;
    crate::validate_plugin_path(executable)?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
#[path = "trusted_executable_linux_tests.rs"]
mod tests;
