//! Explicit deployment records select protected code without copying during startup.
use crate::Error;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    schema_version: u32,
    sha256: String,
    executable_utf16: Vec<u16>,
}

pub(crate) fn resolve() -> Result<PathBuf, Error> {
    let executable = std::env::current_exe()?;
    let registrations = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join(".midnight-explorer").join("plugin-runtimes"));
    resolve_at(&executable, registrations.as_deref())
}

fn resolve_at(executable: &Path, registration_root: Option<&Path>) -> Result<PathBuf, Error> {
    // Inspect the entered namespace before canonicalization can hide a junction.
    let executable = installation_path(executable)?;
    plain_path(&executable)?;
    regular_file(&fs::symlink_metadata(&executable)?)?;
    let direct_error = match trusted_executable(&executable) {
        Ok(()) => return Ok(executable),
        Err(error) => error,
    };
    let digest = executable_digest(&executable)?;
    let Some(registration_root) = registration_root else {
        return missing_registration(executable, direct_error);
    };
    let registration_root = installation_path(registration_root)?;
    if !exists_without_following(&registration_root)? {
        return missing_registration(executable, direct_error);
    }
    protected_path(&registration_root)?;
    if !fs::symlink_metadata(&registration_root)?.is_dir() {
        return Err(Error(
            "Plugin runtime registration root is not a directory".into(),
        ));
    }
    let record = registration_root.join(format!("{digest}.toml"));
    if !exists_without_following(&record)? {
        return missing_registration(executable, direct_error);
    }
    let registration = read_registration(&record)?;
    if registration.schema_version != 1 || registration.sha256 != digest {
        return Err(Error(
            "Plugin runtime registration has an unsupported schema or build hash".into(),
        ));
    }
    let target = installation_path(Path::new(&OsString::from_wide(
        &registration.executable_utf16,
    )))?;
    trusted_executable(&target)?;
    if executable_digest(&target)? != digest {
        return Err(Error(
            "Registered plugin runtime executable does not match this build".into(),
        ));
    }
    Ok(target)
}

fn missing_registration(executable: PathBuf, direct_error: Error) -> Result<PathBuf, Error> {
    let parent = executable
        .parent()
        .ok_or_else(|| Error("Executable has no installation directory".into()))?;
    for directory in ["viewers", "filesystems", "thumbnailers"] {
        if exists_without_following(&parent.join(directory))? {
            return Err(Error(format!(
                "{direct_error}. No private plugin runtime is registered for this build; \
                 run scripts/deploy-private.ps1 -RegisterRuntime for this build and restart."
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

fn trusted_executable(path: &Path) -> Result<(), Error> {
    protected_path(path)?;
    regular_file(&fs::symlink_metadata(path)?)?;
    protected_path(
        path.parent()
            .ok_or_else(|| Error("Executable has no installation directory".into()))?,
    )
}

fn protected_path(path: &Path) -> Result<(), Error> {
    plain_path(path)?;
    crate::validate_plugin_path(path)
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

/// Preserve raw UTF-16 while normalizing namespace syntax without following links.
fn installation_path(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(Error(
            "Plugin runtime path must be absolute without parent traversal".into(),
        ));
    }
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
        return Err(Error("NUL in plugin runtime path".into()));
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
            _ => return Err(Error("Unsupported plugin runtime path namespace".into())),
        },
        _ => {
            return Err(Error(
                "Plugin runtime path has no absolute namespace".into(),
            ));
        }
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
    protected_path(path)?;
    let file = locked_file(path)?;
    let before = crate::native::snapshot(&file)?;
    if before.length > MAX_REGISTRATION_BYTES {
        return Err(Error("Plugin runtime registration exceeds 1 MiB".into()));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_REGISTRATION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REGISTRATION_BYTES {
        return Err(Error("Plugin runtime registration exceeds 1 MiB".into()));
    }
    if before != crate::native::snapshot(&file)? || bytes.len() as u64 != before.length {
        return Err(Error(
            "Plugin runtime registration changed while reading".into(),
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Error("Plugin runtime registration is not UTF-8".into()))?;
    toml::from_str(text)
        .map_err(|error| Error(format!("Invalid plugin runtime registration: {error}")))
}

fn executable_digest(path: &Path) -> Result<String, Error> {
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
    if before != crate::native::snapshot(&file)? || length != before.length {
        return Err(Error(
            "Plugin runtime executable changed while hashing".into(),
        ));
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
#[path = "installation_windows_tests.rs"]
mod tests;
