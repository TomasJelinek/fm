//! Bounded operations with operating-system authorization: a one-shot elevated directory
//! reader and, on Linux, installing the HTML viewer's AppArmor profile ([`SandboxProfile`]).
#![deny(unsafe_op_in_unsafe_fn)]

mod helper;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod native;
mod protocol;
#[cfg(target_os = "linux")]
#[path = "sandbox_profile_linux.rs"]
mod sandbox_profile;
#[cfg(not(target_os = "linux"))]
#[path = "sandbox_profile_other.rs"]
mod sandbox_profile;
pub use sandbox_profile::SandboxProfile;

/// Why installing a [`SandboxProfile`] did not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxProfileError {
    /// The administrator dismissed or refused the authorization; nothing changed.
    Declined,
    /// No trusted system authorization is available (any more), so the profile can only
    /// be installed as described in the system requirements.
    Unavailable,
    /// No polkit authentication agent runs in this session, so nobody can be asked for the
    /// authorization here: asking again cannot help, and the profile can only be installed
    /// as described in the system requirements.
    NoAgent,
    /// The installation could not complete; the text says why.
    Failed(String),
}

use midnight_core::{
    Cancellation, ElevatedDirectoryService, ElevationAvailability, Entry, EntryId, EntryKind,
    FsError,
};
use native::requirement;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fmt,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
    time::{Duration, UNIX_EPOCH},
};

/// SHA-256 of the authorization helper that a build shipped with.
pub type HelperDigest = [u8; 32];

/// Parent adapter. Installation and channel I/O happen only in the listing worker.
///
/// With the shipped helper's digest (recorded in the application executable at build time),
/// a helper is trusted by its content: exactly those bytes, read through a handle that keeps
/// it from changing, in a folder that only the current user and administrators (on Linux,
/// root) can change. Without a digest only an administrator-managed installation qualifies.
#[derive(Default)]
pub struct ElevatedDirectories {
    shipped: Option<HelperDigest>,
    /// A definite availability answer; the installation does not change while it runs.
    availability: OnceLock<ElevationAvailability>,
}

impl ElevatedDirectories {
    /// `shipped`: the SHA-256 of the helper this build shipped with, if the build recorded it.
    pub fn new(shipped: Option<HelperDigest>) -> Self {
        Self {
            shipped,
            availability: OnceLock::new(),
        }
    }
}

impl ElevatedDirectoryService for ElevatedDirectories {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        read(path, cancel, emit, self.shipped.as_ref())
            .map_err(|error| FsError::from_io("authorized directory listing", path, error))
    }

    fn availability(&self) -> ElevationAvailability {
        if let Some(known) = self.availability.get() {
            return known.clone();
        }
        let checked = midnight_plugin_host::installed_executable()
            .map_err(io::Error::other)
            .and_then(|executable| authorization_beside(&executable, self.shipped.as_ref()));
        let definite = checked.as_ref().is_err_and(|error| {
            refusal_cause(error).is_some()
                || matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                )
        });
        let availability = match checked {
            Ok(()) => ElevationAvailability::Available,
            Err(error) => ElevationAvailability::Unavailable {
                reason: unavailable_reason(refusal_cause(&error), self.shipped.is_some()),
                details: error.to_string(),
            },
        };
        // Transient failures, such as a scanner holding the helper open, are asked again.
        if availability == ElevationAvailability::Available || definite {
            let _ = self.availability.set(availability.clone());
        }
        availability
    }
}

/// Every check that an authorized listing runs before it asks for permission, without
/// starting anything: the helper beside `executable`, and on Linux the system's pkexec.
fn authorization_beside(executable: &Path, shipped: Option<&HelperDigest>) -> io::Result<()> {
    helper_beside(executable, shipped)?;
    native::system_authorization()
}

/// Why a helper was refused, for the user-facing explanation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    /// No helper beside the application.
    Missing,
    /// A helper whose content differs from the shipped one.
    Altered,
    /// A helper or folder that another account could change, or that is no ordinary local
    /// file or folder.
    Location,
    /// The system offers no trusted authorization program (Linux pkexec).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    System,
}

#[derive(Debug)]
struct Refusal {
    cause: Cause,
    message: String,
}
impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for Refusal {}

/// A refusal whose text is `message` and whose cause the availability check reports.
pub(crate) fn refusal(kind: io::ErrorKind, cause: Cause, message: String) -> io::Error {
    io::Error::new(kind, Refusal { cause, message })
}
fn refusal_cause(error: &io::Error) -> Option<Cause> {
    error
        .get_ref()?
        .downcast_ref::<Refusal>()
        .map(|refusal| refusal.cause)
}

/// The notice's sentence after "Access to this folder was denied.": why the retry cannot ask
/// for permission and what makes it available.
fn unavailable_reason(cause: Option<Cause>, shipped: bool) -> String {
    let (permission, administrators, protected) = if cfg!(windows) {
        (
            "administrator",
            "administrators",
            "only administrators can change it, such as C:\\Program Files (the per-machine \
             installer does this)",
        )
    } else {
        (
            "root",
            "root",
            "only root can change it, such as /opt/midnight-explorer installed with sudo",
        )
    };
    let because = match cause {
        Some(Cause::System) => {
            return format!(
                "Opening it needs {permission} permission, which this system cannot ask for: \
                 the polkit program pkexec is missing or not owned by root."
            );
        }
        Some(Cause::Missing) => format!(
            "its authorization helper ({HELPER}) is missing. Reinstall Midnight Explorer to \
             restore it."
        ),
        Some(Cause::Altered) => format!(
            "its authorization helper ({HELPER}) is not the one this version shipped with. \
             Reinstall Midnight Explorer to restore it."
        ),
        Some(Cause::Location) if shipped => format!(
            "other accounts on this computer can change its installation folder. Install \
             Midnight Explorer where only you and {administrators} can change it."
        ),
        Some(Cause::Location) => format!("Midnight Explorer must be installed where {protected}."),
        None => "its authorization helper could not be checked.".into(),
    };
    format!(
        "Opening it needs {permission} permission, which this installation cannot ask for: {because}"
    )
}

/// The largest helper whose digest is computed; the shipped helper is far smaller.
const MAX_HELPER_BYTES: u64 = 64 << 20;

/// Read the helper through `file`, an open handle that keeps it from changing, and refuse it
/// unless its content is the shipped helper's.
pub(crate) fn verify_shipped(
    file: &mut impl Read,
    path: &Path,
    shipped: &HelperDigest,
    requirement: &str,
) -> io::Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        total += read as u64;
        if total > MAX_HELPER_BYTES {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if total > MAX_HELPER_BYTES || hasher.finalize().as_slice() != shipped.as_slice() {
        return Err(refusal(
            io::ErrorKind::PermissionDenied,
            Cause::Altered,
            format!(
                "{requirement} {} is not the authorization helper this version shipped with.",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// The digest in its 64-digit hexadecimal spelling, as the build records it; `None` for any
/// other text.
pub fn parse_helper_digest(text: &str) -> Option<HelperDigest> {
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

fn read(
    path: &Path,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Entry) -> bool,
    shipped: Option<&HelperDigest>,
) -> io::Result<()> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    if !path.is_absolute() {
        return Err(protocol::invalid("Directory path must be absolute"));
    }
    let helper = trusted_helper(shipped)?;
    let request = protocol::Request {
        path: native::encode_path(path),
    };
    let reply = native::list(&helper, &request, cancel, shipped)?;
    let entries = reply.map_err(|error| {
        io::Error::new(
            if error.denied {
                io::ErrorKind::PermissionDenied
            } else {
                io::ErrorKind::Other
            },
            error.message,
        )
    })?;
    for (index, wire) in entries.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let name = native::decode_path(&wire.name)?;
        let mut components = name.components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name.as_os_str().is_empty()
            || name.file_name() != Some(name.as_os_str())
        {
            return Err(protocol::invalid("Helper returned a non-basename entry"));
        }
        let kind = match wire.kind {
            0 => EntryKind::Directory,
            1 => EntryKind::DirectoryLink,
            2 => EntryKind::File,
            3 => EntryKind::Symlink,
            4 => EntryKind::Other,
            _ => return Err(protocol::invalid("Helper returned an unknown entry kind")),
        };
        let mut entry = Entry::new(EntryId(index as u64 + 1), path.join(name), kind);
        entry.kind = kind;
        entry.metadata.size = wire.size;
        entry.metadata.modified = wire.modified.and_then(|(seconds, nanos)| {
            if seconds >= 0 {
                UNIX_EPOCH.checked_add(Duration::new(seconds as u64, nanos))
            } else {
                UNIX_EPOCH
                    .checked_sub(Duration::from_secs(seconds.unsigned_abs()))
                    .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
            }
        });
        entry.metadata.native_attributes = wire.native_attributes;
        entry.metadata.unix_mode = wire.unix_mode;
        if !emit(entry) {
            return Err(cancelled());
        }
    }
    Ok(())
}
const HELPER: &str = if cfg!(windows) {
    "midnight-elevated-listing.exe"
} else {
    "midnight-elevated-listing"
};

/// Locate the helper beside the installed executable and accept it only when no account but
/// the current user and administrators (root) can change it, and, when the build recorded the
/// shipped helper's digest, only with that content. Without a digest only administrators
/// (root) may be able to change it. It runs as Administrator/root: another account that could
/// replace it, or plant a library beside it, would turn one consent prompt into its own code.
fn trusted_helper(shipped: Option<&HelperDigest>) -> io::Result<PathBuf> {
    let executable = midnight_plugin_host::installed_executable().map_err(io::Error::other)?;
    helper_beside(&executable, shipped)
}
/// Apply that policy to the helper beside `executable`.
fn helper_beside(executable: &Path, shipped: Option<&HelperDigest>) -> io::Result<PathBuf> {
    let parent = executable
        .parent()
        .ok_or_else(|| protocol::invalid("Missing installation"))?;
    let helper = parent.join(HELPER);
    for component in helper.ancestors() {
        let metadata = match std::fs::symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound && component == helper => {
                return Err(refusal(
                    io::ErrorKind::NotFound,
                    Cause::Missing,
                    format!(
                        "The authorization helper {HELPER} is not installed beside Midnight \
                         Explorer in {}. {}",
                        parent.display(),
                        requirement(shipped.is_some())
                    ),
                ));
            }
            Err(error) => return Err(error),
        };
        // Canonicalization below must not resolve a link. Any other reparse point is judged
        // by the platform check on pinned handles (Windows accepts only storage-only tags).
        if metadata.file_type().is_symlink() {
            return Err(refusal(
                io::ErrorKind::PermissionDenied,
                Cause::Location,
                format!(
                    "{} {} is a link.",
                    requirement(shipped.is_some()),
                    component.display()
                ),
            ));
        }
    }
    if !std::fs::metadata(&helper)?.is_file() {
        return Err(refusal(
            io::ErrorKind::InvalidData,
            Cause::Location,
            format!(
                "{} {} is not a regular file.",
                requirement(shipped.is_some()),
                helper.display()
            ),
        ));
    }
    let helper = helper.canonicalize()?;
    native::protected_helper(&helper, shipped)?;
    Ok(helper)
}
pub(crate) fn cancelled() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Authorized directory listing cancelled",
    )
}
/// Dedicated helper entry point; startup arguments identify only the authenticated local channel.
#[doc(hidden)]
pub fn serve(args: Vec<OsString>) -> io::Result<()> {
    native::serve(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_helper_is_refused_with_the_installation_requirement() {
        // Per-user deployments omit the helper. The crate directory exists without one, so
        // the lookup reaches the missing helper without creating or changing anything.
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!directory.join(HELPER).exists());
        let error = helper_beside(&directory.join("midnight-explorer"), None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        let message = error.to_string();
        let expected_directory = directory.display().to_string();
        assert!(message.contains(HELPER), "{message}");
        assert!(message.contains(&expected_directory), "{message}");
        assert!(message.ends_with(requirement(false)), "{message}");
        // The same requirement that a refused, unprotected helper location explains.
        let (installer, location) = if cfg!(windows) {
            ("only administrators can change it", r"C:\Program Files")
        } else {
            ("only root can change it", "/opt/midnight-explorer")
        };
        assert!(message.contains(installer), "{message}");
        assert!(message.contains(location), "{message}");
    }

    #[test]
    fn unprotected_helper_refusal_states_the_same_requirement() {
        // Locations that each platform refuses without changing anything: Windows refuses a
        // network path before opening it, and Linux refuses the root folder, which is no
        // executable file.
        let refused = Path::new(if cfg!(windows) {
            r"\\server\share\midnight-elevated-listing.exe"
        } else {
            "/"
        });
        let error = native::protected_helper(refused, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        // The missing-helper message ends with the sentence that this refusal begins with.
        let message = error.to_string();
        assert!(message.starts_with(requirement(false)), "{message}");
    }

    #[test]
    fn helper_digest_spelling_is_exact() {
        let text = "00ff".repeat(16);
        let digest = parse_helper_digest(&text).unwrap();
        assert_eq!(digest[0], 0);
        assert_eq!(digest[1], 0xff);
        assert_eq!(parse_helper_digest(&format!("{text}\n")), Some(digest));
        for refused in ["", "00ff", &format!("{text}0"), &"zz".repeat(32)] {
            assert_eq!(parse_helper_digest(refused), None, "{refused}");
        }
    }

    #[test]
    fn shipped_digest_accepts_the_shipped_bytes_and_refuses_any_other() {
        let shipped: HelperDigest = Sha256::digest(b"shipped helper").into();
        let path = Path::new("helper");
        verify_shipped(&mut &b"shipped helper"[..], path, &shipped, "Requirement.").unwrap();
        for altered in [
            &b"shipped helpe"[..],
            b"shipped helper!",
            b"Shipped helper",
            b"",
        ] {
            let error =
                verify_shipped(&mut &altered[..], path, &shipped, "Requirement.").unwrap_err();
            assert_eq!(refusal_cause(&error), Some(Cause::Altered));
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            let message = error.to_string();
            assert!(
                message.starts_with("Requirement. helper is not"),
                "{message}"
            );
        }
    }

    /// A test installation in the build directory: the executable and the helper as the
    /// current user created them. The build folder inherits write access for other accounts
    /// on its drive (on Linux it is user-owned and the helper is no root-owned file there
    /// either), so the retry is unavailable, and the notice says why without starting
    /// anything.
    #[test]
    fn availability_of_a_test_installation_names_the_refusal() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("midnight-explorer");
        let missing = authorization_beside(&executable, None).unwrap_err();
        assert_eq!(refusal_cause(&missing), Some(Cause::Missing));
        let reason = unavailable_reason(refusal_cause(&missing), true);
        assert!(reason.contains("is missing. Reinstall"), "{reason}");

        let helper = root.path().join(HELPER);
        std::fs::write(&helper, b"test helper").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let shipped: HelperDigest = Sha256::digest(b"test helper").into();
        for digest in [None, Some(&shipped)] {
            let error = authorization_beside(&executable, digest).unwrap_err();
            let cause = refusal_cause(&error);
            let details = error.to_string();
            let reason = unavailable_reason(cause, digest.is_some());
            if cfg!(windows) {
                // Without a digest the user-owned helper itself is refused first; with one,
                // the access that other accounts inherit on this drive.
                assert_eq!(cause, Some(Cause::Location), "{details}");
                assert!(
                    details.starts_with(requirement(digest.is_some())),
                    "{details}"
                );
                if digest.is_some() {
                    assert!(details.contains("non-administrative"), "{details}");
                    assert!(
                        reason.contains("other accounts on this computer"),
                        "{reason}"
                    );
                } else {
                    assert!(details.contains("is not owned by SYSTEM"), "{details}");
                    assert!(reason.contains("C:\\Program Files"), "{reason}");
                    assert!(reason.contains("per-machine installer"), "{reason}");
                }
            } else {
                assert!(
                    matches!(cause, Some(Cause::Location | Cause::System)),
                    "{details}"
                );
            }
            assert!(reason.starts_with("Opening it needs "), "{reason}");
        }
        // An altered helper: the digest refuses it wherever the folder would be accepted.
        let other: HelperDigest = Sha256::digest(b"another helper").into();
        let error = verify_shipped(
            &mut std::fs::File::open(&helper).unwrap(),
            &helper,
            &other,
            requirement(true),
        )
        .unwrap_err();
        assert_eq!(refusal_cause(&error), Some(Cause::Altered));
        let reason = unavailable_reason(Some(Cause::Altered), true);
        assert!(
            reason.contains("not the one this version shipped with"),
            "{reason}"
        );
    }

    #[test]
    fn availability_is_remembered_once_definite() {
        let service = ElevatedDirectories::new(None);
        let first = service.availability();
        // The test executable has no helper beside it: a definite answer, remembered.
        assert!(
            matches!(first, ElevationAvailability::Unavailable { .. }),
            "{first:?}"
        );
        assert_eq!(service.availability.get(), Some(&first));
        assert_eq!(service.availability(), first);
    }

    /// Cargo.toml sets `test = false` for the `midnight-elevated-listing` binary, so
    /// `cargo test` builds no harness for it and a unit test there would never run.
    #[test]
    fn listing_binary_has_no_unit_tests() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let source = std::fs::read_to_string(&path).expect("binary source");
        for line in source.lines().map(str::trim) {
            let words: Vec<_> = line
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .collect();
            let attribute = line.starts_with("#[") || line.starts_with("#![");
            // A module file would hold code that this check does not read.
            let module_file =
                !line.starts_with("//") && words.contains(&"mod") && line.ends_with(';');
            assert!(
                !(attribute && words.contains(&"test")) && !module_file,
                "{}: `{line}`; remove `test = false` from the binary in Cargo.toml",
                path.display()
            );
        }
    }
}
