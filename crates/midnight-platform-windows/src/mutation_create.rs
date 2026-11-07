//! Explicit creation walks native directory handles without following entered links.
use super::{changed, checked, failure, invalid, native, validate_name, validate_path};
use midnight_core::{Cancellation, FsError, PreparedDirectory};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Component, Path, PathBuf, Prefix},
    sync::Arc,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_OPEN},
    Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_TRAVERSE, SYNCHRONIZE,
    },
};

struct WindowsDirectory {
    path: PathBuf,
    directory: File,
    identity: native::Identity,
}

pub(super) fn prepare(
    path: &Path,
    cancel: &Cancellation,
) -> Result<Arc<dyn PreparedDirectory>, FsError> {
    checked(cancel, path)?;
    validate_path(path, true)?;
    let directory =
        native::open_parent(path).map_err(|e| failure("prepare create directory", path, e))?;
    if !directory
        .metadata()
        .map_err(|e| failure("read base directory", path, e))?
        .is_dir()
    {
        return Err(failure(
            "prepare create directory",
            path,
            io::Error::new(io::ErrorKind::NotADirectory, "the base must be a directory"),
        ));
    }
    let identity =
        native::identity(&directory).map_err(|e| failure("read base identity", path, e))?;
    checked(cancel, path)?;
    Ok(Arc::new(WindowsDirectory {
        path: path.to_owned(),
        directory,
        identity,
    }))
}

impl WindowsDirectory {
    fn revalidate(&self) -> Result<(), FsError> {
        let visible = native::open_parent(&self.path)
            .map_err(|e| failure("revalidate base directory", &self.path, e))?;
        if native::identity(&visible).map_err(|e| failure("read base identity", &self.path, e))?
            != self.identity
        {
            return Err(changed(&self.path));
        }
        Ok(())
    }

    fn cursor(&self, input: &Path, cancel: &Cancellation) -> Result<Cursor, FsError> {
        let mut components = input.components();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix)) => Some(prefix),
            _ => None,
        };
        if let Some(prefix) = prefix {
            if !matches!(
                prefix.kind(),
                Prefix::Disk(_)
                    | Prefix::VerbatimDisk(_)
                    | Prefix::UNC(_, _)
                    | Prefix::VerbatimUNC(_, _)
            ) {
                return Err(invalid(
                    input,
                    "device and special namespaces are not accepted",
                ));
            }
            if !input.has_root() {
                let base_prefix = self.path.components().next();
                let same_drive = match (prefix.kind(), base_prefix) {
                    (Prefix::Disk(requested), Some(Component::Prefix(base))) => {
                        matches!(base.kind(), Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) if drive.eq_ignore_ascii_case(&requested))
                    }
                    _ => false,
                };
                if !same_drive {
                    return Err(invalid(
                        input,
                        "a drive-relative path must use the captured panel's drive; enter an absolute drive path instead",
                    ));
                }
            }
        }
        if input.has_root() {
            let root = if let Some(prefix) = prefix {
                // Build the root of the validated drive/share without path replacement semantics.
                let mut root = prefix.as_os_str().to_os_string();
                root.push("\\");
                PathBuf::from(root)
            } else {
                self.path
                    .components()
                    .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
                    .collect()
            };
            return Cursor::root(root);
        }
        if input.components().any(|part| part == Component::ParentDir) {
            // A navigated panel can itself be a junction. Resolve that captured base,
            // not the user's requested components, then pin its entire ancestor chain.
            // Verify the terminal identity before the first creation. Holding each
            // ancestor without delete sharing keeps `..` tied to this chain.
            let canonical = std::fs::canonicalize(&self.path)
                .map_err(|e| failure("resolve base directory", &self.path, e))?;
            let root = canonical
                .components()
                .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
                .collect();
            let mut cursor = Cursor::root(root)?;
            for component in canonical.components() {
                if let Component::Normal(name) = component {
                    checked(cancel, &cursor.path)?;
                    cursor.path.push(name);
                    let child = open_directory(cursor.current(), name)
                        .map_err(|e| failure("pin base ancestor", &cursor.path, e))?;
                    cursor.directories.push(child);
                }
            }
            if native::identity(cursor.current())
                .map_err(|e| failure("read base identity", &self.path, e))?
                != self.identity
            {
                return Err(changed(&self.path));
            }
            Ok(cursor)
        } else {
            Ok(Cursor {
                path: self.path.clone(),
                directories: vec![
                    self.directory
                        .try_clone()
                        .map_err(|e| failure("retain base directory", &self.path, e))?,
                ],
            })
        }
    }
}

struct Cursor {
    path: PathBuf,
    directories: Vec<File>,
}

impl Cursor {
    fn root(path: PathBuf) -> Result<Self, FsError> {
        let directory =
            native::open_parent(&path).map_err(|e| failure("open target root", &path, e))?;
        Ok(Self {
            path,
            directories: vec![directory],
        })
    }

    fn current(&self) -> &File {
        self.directories
            .last()
            .expect("cursor always retains its root")
    }

    fn parent(&mut self) {
        if self.directories.len() > 1 {
            self.directories.pop();
            self.path.pop();
        }
    }
}

fn open_directory(parent: &File, name: &OsStr) -> io::Result<File> {
    let directory = native::open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )?;
    if directory.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "creating through a junction or symbolic link is not allowed",
        ));
    }
    Ok(directory)
}

impl PreparedDirectory for WindowsDirectory {
    fn create_file(&self, name: &OsStr, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        checked(cancel, &self.path)?;
        validate_name(name).map_err(|e| failure("validate create empty file", &self.path, e))?;
        self.revalidate()?;
        let result = self.path.join(name);
        checked(cancel, &result)?;
        let file = native::create_empty_file(&self.directory, name)
            .map_err(|e| failure("create empty file", &result, e))?;
        drop(file);
        Ok(result)
    }

    fn create(&self, target: &Path, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        checked(cancel, &self.path)?;
        // Normalize separators only, preserving raw UTF-16 and all parent steps.
        // Check the raw final component because Path::components omits trailing '.'.
        let units: Vec<_> = target
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
        let final_name = units
            .split(|unit| *unit == b'\\' as u16)
            .rfind(|part| !part.is_empty());
        if final_name.is_none_or(|name| name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16])
        {
            return Err(invalid(
                target,
                "enter a path ending in a new directory name",
            ));
        }
        let input = PathBuf::from(OsString::from_wide(&units));
        let components: Vec<_> = input.components().collect();
        if !matches!(components.last(), Some(Component::Normal(_))) {
            return Err(invalid(
                target,
                "enter a path ending in a new directory name",
            ));
        }
        // Validate the entire request before creating even the first component.
        for component in &components {
            if let Component::Normal(name) = component {
                validate_name(name).map_err(|e| failure("validate create directory", target, e))?;
            }
        }
        self.revalidate()?;
        let mut cursor = self.cursor(&input, cancel)?;
        for (index, component) in components.iter().enumerate() {
            checked(cancel, &cursor.path)?;
            let name = match component {
                Component::Normal(name) => name,
                Component::ParentDir => {
                    cursor.parent();
                    continue;
                }
                _ => continue,
            };
            cursor.path.push(name);
            let directory =
                native::create_directory(cursor.current(), name, index + 1 == components.len())
                    .map_err(|e| {
                        let mut error = failure("create directory", &cursor.path, e);
                        error.message.push_str(
                            "; any intermediate directories already created are retained",
                        );
                        error
                    })?;
            cursor.directories.push(directory);
        }
        Ok(cursor.path)
    }
}
