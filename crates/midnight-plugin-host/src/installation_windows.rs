//! Explicit deployment records select protected code without copying during startup.
use crate::Error;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
};

const MAX_REGISTRATION_BYTES: u64 = 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
#[path = "installation_windows_path.rs"]
mod launch_path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeVerification {
    Disabled,
    Sha256,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherPointer {
    schema_version: u32,
    launch_executable_utf16: Vec<u16>,
    launch_length: u64,
    launch_last_write_time_100ns: i64,
    registration_sha256: String,
}

fn verification_from_env(value: Option<&OsStr>) -> Result<RuntimeVerification, Error> {
    match value {
        None => Ok(RuntimeVerification::Disabled),
        Some(value) if value == OsStr::new("0") => Ok(RuntimeVerification::Disabled),
        Some(value) if value == OsStr::new("1") => Ok(RuntimeVerification::Sha256),
        Some(_) => Err(Error(
            "MIDNIGHT_VERIFY_RUNTIME_SHA256 must be exactly 0 or 1; unset defaults to 0".into(),
        )),
    }
}

fn launch_path_key(normalized_utf16: &[u16]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"midnight-launch-path-v1\0");
    for unit in normalized_utf16 {
        hash.update(unit.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    schema_version: u32,
    sha256: String,
    executable_utf16: Vec<u16>,
}

#[derive(Clone, Copy)]
enum ExecutableRole {
    Launch,
    RegisteredTarget,
}

pub(crate) fn resolve() -> Result<PathBuf, Error> {
    let verification =
        verification_from_env(std::env::var_os("MIDNIGHT_VERIFY_RUNTIME_SHA256").as_deref())?;
    let executable = std::env::current_exe()?;
    let registrations = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join(".midnight-explorer").join("plugin-runtimes"));
    resolve_at(&executable, registrations.as_deref(), verification)
}

fn resolve_at(
    executable: &Path,
    registration_root: Option<&Path>,
    verification: RuntimeVerification,
) -> Result<PathBuf, Error> {
    resolve_at_with_digest(
        executable,
        registration_root,
        verification,
        executable_digest,
    )
}

fn resolve_at_with_digest(
    executable: &Path,
    registration_root: Option<&Path>,
    verification: RuntimeVerification,
    mut digest: impl FnMut(&Path, ExecutableRole) -> Result<String, Error>,
) -> Result<PathBuf, Error> {
    // Inspect the entered namespace before any native normalization can hide a junction.
    let executable = runtime_path(executable)?;
    plain_path(&executable)?;
    regular_file(&fs::symlink_metadata(&executable)?)?;
    // The process reports the spelling used at launch. Letter case, 8.3, SUBST and
    // mapped-drive aliases name the same file, so trust is decided for the handle's
    // native final spelling (both spellings are reparse-checked by `open`). Every
    // later package path then derives from this canonical directory.
    let (executable, launch, direct) = match launch_path::open(&executable) {
        Ok(launch) => match trusted_executable(&launch.executable) {
            Ok(()) => return Ok(launch.executable),
            Err(error) => {
                // Other accounts' permission entries alone do not refuse the installation:
                // they could replace this executable too. A registered private deployment
                // still takes precedence below; otherwise the installation runs as it is and
                // the application reports it as shared.
                let shared = shared_executable(&launch.executable).is_ok();
                (
                    launch.executable.clone(),
                    Some(launch),
                    Direct { error, shared },
                )
            }
        },
        // Fallbacks keep their previous entered-spelling behavior.
        Err(error) => (
            executable,
            None,
            Direct {
                error,
                shared: false,
            },
        ),
    };
    match verification {
        RuntimeVerification::Disabled => {
            resolve_by_pointer(executable, launch, registration_root, direct)
        }
        RuntimeVerification::Sha256 => {
            drop(launch);
            resolve_by_sha256(executable, registration_root, direct, &mut digest)
        }
    }
}

/// Why the launch executable was not trusted directly, and whether only other accounts'
/// permission entries were the reason.
struct Direct {
    error: Error,
    shared: bool,
}

fn registration_directory(registration_root: Option<&Path>) -> Result<Option<PathBuf>, Error> {
    let Some(registration_root) = registration_root else {
        return Ok(None);
    };
    let registration_root = runtime_path(registration_root)?;
    if !exists_without_following(&registration_root)? {
        return Ok(None);
    }
    protected_path(&registration_root)?;
    if !fs::symlink_metadata(&registration_root)?.is_dir() {
        return Err(Error(
            "Plugin runtime registration root is not a directory".into(),
        ));
    }
    Ok(Some(registration_root))
}

fn resolve_by_sha256(
    executable: PathBuf,
    registration_root: Option<&Path>,
    direct: Direct,
    digest: &mut impl FnMut(&Path, ExecutableRole) -> Result<String, Error>,
) -> Result<PathBuf, Error> {
    // Opt-in preserves the original order, content reads and snapshot verification.
    let expected_digest = digest(&executable, ExecutableRole::Launch)?;
    let Some(registration_root) = registration_directory(registration_root)? else {
        return missing_registration(executable, direct);
    };
    let record = registration_root.join(format!("{expected_digest}.toml"));
    if !exists_without_following(&record)? {
        return missing_registration(executable, direct);
    }
    let target = registered_target(&record, &expected_digest)?;
    if digest(&target, ExecutableRole::RegisteredTarget)? != expected_digest {
        return Err(Error(
            "Registered plugin runtime executable does not match this build".into(),
        ));
    }
    Ok(target)
}

fn resolve_by_pointer(
    executable: PathBuf,
    launch: Option<launch_path::LaunchFile>,
    registration_root: Option<&Path>,
    direct: Direct,
) -> Result<PathBuf, Error> {
    let Some(registration_root) = registration_directory(registration_root)? else {
        return missing_launcher_pointer(executable, direct);
    };
    let launchers = registration_root.join("launchers");
    if !exists_without_following(&launchers)? {
        return missing_launcher_pointer(executable, direct);
    }
    protected_path(&launchers)?;
    if !fs::symlink_metadata(&launchers)?.is_dir() {
        return Err(Error(
            "Plugin runtime launchers path is not a directory".into(),
        ));
    }
    // This retained metadata-only handle supplies the native final path and both hints.
    let launch = match launch {
        Some(launch) => launch,
        None => launch_path::open(&executable)?,
    };
    let pointer_path = launchers.join(format!("{}.toml", launch_path_key(&launch.utf16)));
    if !exists_without_following(&pointer_path)? {
        return missing_launcher_pointer(executable, direct);
    }
    let pointer = read_launcher_pointer(&pointer_path)?;
    if pointer.schema_version != 1 || !lowercase_sha256(&pointer.registration_sha256) {
        return Err(Error(
            "Plugin runtime launcher pointer has an unsupported schema or invalid registration \
             hash; register this build's private runtime again"
                .into(),
        ));
    }
    if pointer.launch_executable_utf16 != launch.utf16
        || pointer.launch_length != launch.length
        || pointer.launch_last_write_time_100ns != launch.last_write_time_100ns
    {
        return missing_deployment(
            executable,
            direct,
            &format!(
                "The protected plugin runtime launcher pointer for {:?} is stale",
                launch.executable
            ),
        );
    }
    let record = registration_root.join(format!("{}.toml", pointer.registration_sha256));
    if !exists_without_following(&record)? {
        return missing_launcher_pointer(executable, direct);
    }
    let target = registered_target(&record, &pointer.registration_sha256)?;
    drop(launch);
    Ok(target)
}

fn lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn registered_target(record: &Path, expected_digest: &str) -> Result<PathBuf, Error> {
    let registration = read_registration(record)?;
    if registration.schema_version != 1 || registration.sha256 != expected_digest {
        return Err(Error(
            "Plugin runtime registration has an unsupported schema or build hash".into(),
        ));
    }
    let target = runtime_path(Path::new(&OsString::from_wide(
        &registration.executable_utf16,
    )))?;
    trusted_executable(&target)?;
    Ok(target)
}

fn missing_registration(executable: PathBuf, direct: Direct) -> Result<PathBuf, Error> {
    missing_deployment(
        executable,
        direct,
        "No private plugin runtime is registered for this build",
    )
}

fn missing_launcher_pointer(executable: PathBuf, direct: Direct) -> Result<PathBuf, Error> {
    missing_deployment(
        executable,
        direct,
        "A protected plugin runtime launcher pointer is missing or stale",
    )
}

/// No registered private deployment applies. A shared installation runs as it is; an
/// installation refused for another reason (another account's object, an unsupported
/// permission entry) runs only without plugin folders. `reason` names the missing
/// registration for diagnostics; end users never register one.
fn missing_deployment(executable: PathBuf, direct: Direct, reason: &str) -> Result<PathBuf, Error> {
    if direct.shared {
        return Ok(executable);
    }
    let parent = executable
        .parent()
        .ok_or_else(|| Error("Executable has no installation directory".into()))?;
    for directory in ["viewers", "filesystems", "thumbnailers"] {
        if exists_without_following(&parent.join(directory))? {
            return Err(Error(format!(
                "Midnight Explorer cannot load its plugins from {}: {}. Extract the complete \
                 download again into a folder of your own, for example in your user profile \
                 ({reason}).",
                parent.display(),
                direct.error
            )));
        }
    }
    // A standalone executable has no plugin code to discover. All loaders still
    // validate any packages that appear after this process-wide choice is cached.
    Ok(executable)
}

fn exists_without_following(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// An executable that only the user, SYSTEM and Administrators can change, in such a folder.
fn trusted_executable(path: &Path) -> Result<(), Error> {
    executable_with(path, protected_path)
}

/// [`trusted_executable`] that accepts other accounts' permission entries.
fn shared_executable(path: &Path) -> Result<(), Error> {
    executable_with(path, |path| {
        plain_path(path)?;
        crate::validate_plugin_path(path)
    })
}

fn executable_with(path: &Path, check: impl Fn(&Path) -> Result<(), Error>) -> Result<(), Error> {
    check(path)?;
    regular_file(&fs::symlink_metadata(path)?)?;
    check(
        path.parent()
            .ok_or_else(|| Error("Executable has no installation directory".into()))?,
    )
}

/// Registrations and the deployments they select confer authority: they must be private.
fn protected_path(path: &Path) -> Result<(), Error> {
    plain_path(path)?;
    crate::validate_private_path(path)
}

fn plain_path(path: &Path) -> Result<(), Error> {
    for (index, component) in path.ancestors().enumerate() {
        let metadata = fs::symlink_metadata(component)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(Error(format!(
                "Plugin runtime path contains a reparse point: {}",
                component.display()
            )));
        }
        if index != 0 && !metadata.is_dir() {
            return Err(Error(
                "Plugin runtime path ancestor is not a directory".into(),
            ));
        }
    }
    Ok(())
}

/// The absolute spelling of an installation path that trust validation inspects, in native
/// extended syntax (`\\?\C:\...`, `\\?\UNC\...`) with its raw UTF-16 preserved. A relative
/// path is resolved against the current directory by `std::path::absolute`, which follows no
/// link; a verbatim path stays unchanged. Device and other namespaces are refused.
pub fn installation_path(path: &Path) -> io::Result<PathBuf> {
    verbatim(&std::path::absolute(path)?)
}

/// The launch path and recorded deployment paths must already be absolute, without parent
/// traversal: recorded data is never resolved against the current directory.
fn runtime_path(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(Error(
            "Plugin runtime path must be absolute without parent traversal".into(),
        ));
    }
    verbatim(path).map_err(Error::from)
}

/// Preserve raw UTF-16 while normalizing namespace syntax without following links.
fn verbatim(path: &Path) -> io::Result<PathBuf> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
    let mut wide: Vec<_> = path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .collect();
    if wide.contains(&0) {
        return Err(invalid("NUL in installation path"));
    }
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => {}
            Prefix::Disk(_) => {
                wide.splice(..0, "\\\\?\\".encode_utf16());
            }
            Prefix::UNC(_, _) => {
                wide.splice(..2, "\\\\?\\UNC\\".encode_utf16());
            }
            _ => return Err(invalid("Unsupported installation path namespace")),
        },
        _ => return Err(invalid("Installation path has no absolute namespace")),
    }
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

fn regular_file(metadata: &fs::Metadata) -> Result<(), Error> {
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error(
            "Plugin runtime requires a regular non-reparse file".into(),
        ));
    }
    Ok(())
}

fn locked_file(path: &Path) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    regular_file(&file.metadata()?)?;
    Ok(file)
}

fn read_registration(path: &Path) -> Result<Registration, Error> {
    read_record(path, "registration")
}

fn read_launcher_pointer(path: &Path) -> Result<LauncherPointer, Error> {
    read_record(path, "launcher pointer")
}

fn read_record<T: serde::de::DeserializeOwned>(path: &Path, kind: &str) -> Result<T, Error> {
    protected_path(path)?;
    let file = locked_file(path)?;
    let before = crate::native::snapshot(&file)?;
    if before.length > MAX_REGISTRATION_BYTES {
        return Err(Error(format!("Plugin runtime {kind} exceeds 1 MiB")));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_REGISTRATION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REGISTRATION_BYTES {
        return Err(Error(format!("Plugin runtime {kind} exceeds 1 MiB")));
    }
    if before != crate::native::snapshot(&file)? || bytes.len() as u64 != before.length {
        return Err(Error(format!(
            "Plugin runtime {kind} changed while reading"
        )));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Error(format!("Plugin runtime {kind} is not UTF-8")))?;
    toml::from_str(text).map_err(|error| Error(format!("Invalid plugin runtime {kind}: {error}")))
}

fn executable_digest(path: &Path, role: ExecutableRole) -> Result<String, Error> {
    let mut file = locked_file(path)?;
    let before = crate::native::snapshot(&file)?;
    if before.length > MAX_EXECUTABLE_BYTES {
        return Err(Error("Plugin runtime executable exceeds 512 MiB".into()));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut length = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        if length > MAX_EXECUTABLE_BYTES {
            return Err(Error("Plugin runtime executable exceeds 512 MiB".into()));
        }
        hash.update(&buffer[..count]);
    }
    let after = crate::native::snapshot(&file)?;
    validate_executable_snapshot(path, role, &before, &after, length)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn validate_executable_snapshot(
    path: &Path,
    role: ExecutableRole,
    before: &crate::native::Snapshot,
    after: &crate::native::Snapshot,
    bytes_read: u64,
) -> Result<(), Error> {
    if before != after || bytes_read != before.length {
        let role = match role {
            ExecutableRole::Launch => "launch_executable",
            ExecutableRole::RegisteredTarget => "registered_target",
        };
        return Err(Error(format!(
            "Plugin runtime executable changed while hashing: \
             role={role}; path={path:?}; changed_fields=[{}]; \
             bytes_read={bytes_read}; expected_length={}",
            before.changed_fields(after),
            before.length,
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "installation_windows_tests.rs"]
mod tests;
