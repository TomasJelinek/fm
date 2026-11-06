//! Freedesktop home trash, on the source device only. Metadata is reserved with
//! O_EXCL and flushed before the no-replace move into files/. No copying or
//! permanent-delete fallback is performed. All trash directories are opened
//! without symlinks and must be owned by the effective uid with mode 0700.
//! Their identities are excluded from trash operations before reservation.
//! See https://specifications.freedesktop.org/trash/1.0/ (sections 5 and 6).
//! The two-directory update is not crash-atomic: interrupted operations can
//! leave orphan metadata or private staging, never an intentional data purge.
use super::*;
use std::{io::Write, os::unix::ffi::OsStringExt, path::Component};

pub(super) fn home_trash(home: Option<&Path>, data_home: Option<OsString>) -> Option<PathBuf> {
    if let Some(data_home) = data_home.filter(|value| Path::new(value).is_absolute()) {
        return Some(PathBuf::from(data_home).join("Trash"));
    }
    home.filter(|path| path.is_absolute())
        .map(|home| home.join(".local/share/Trash"))
}

pub(super) fn move_entry(
    prepared: &Prepared,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Path),
) -> Result<(), FsError> {
    let root = prepared.trash_root.as_ref().ok_or_else(|| {
        error(
            &prepared.item.path,
            ErrorKind::Unsupported,
            "home trash is unavailable",
        )
    })?;
    check_cancel(cancel, &prepared.item.path)?;
    let trash = Trash::open(root, prepared.identity)?;
    if Identity::of(&trash.files, root)?.device != prepared.identity.device {
        return Err(error(
            &prepared.item.path,
            ErrorKind::Unsupported,
            "home trash is on another filesystem; cross-filesystem trash is unsupported",
        ));
    }
    let reservation = trash.reserve(prepared)?;
    let staged = match Staged::create(prepared) {
        Ok(staged) => staged,
        Err(failure) => return Err(reservation.remove(&trash, failure)),
    };
    if let Err(failure) = staged.capture(prepared, cancel) {
        return Err(reservation.remove(&trash, failure));
    }
    let result = check_cancel(cancel, &prepared.item.path).and_then(|()| {
        prepared.identity.verify(
            &staged.directory,
            OsStr::new(STAGED_NAME),
            &prepared.item.path,
        )?;
        renameat_with(
            &*staged.directory,
            STAGED_NAME,
            &trash.files,
            &reservation.name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| {
            if e == rustix::io::Errno::XDEV {
                error(
                    &prepared.item.path,
                    ErrorKind::Unsupported,
                    "trash crosses a mount boundary; cross-filesystem trash is unsupported",
                )
            } else {
                native_error(&prepared.item.path, e)
            }
        })
    });
    match result {
        Ok(()) => {
            progress(&prepared.item.path);
            staged.cleanup()
        }
        Err(failure) => Err(reservation.remove(&trash, staged.restore(prepared, failure))),
    }
}

struct Trash {
    files: File,
    info: File,
    path: PathBuf,
}

impl Trash {
    fn open(path: &Path, selected: Identity) -> Result<Self, FsError> {
        if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            return Err(error(
                path,
                ErrorKind::InvalidInput,
                "trash requires an absolute native path",
            ));
        }
        let mut directory = File::from(
            openat(CWD, "/", DIRECTORY_FLAGS, Mode::empty()).map_err(|e| native_error(path, e))?,
        );
        let mut current_path = PathBuf::from("/");
        for component in path.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current_path.push(name);
            // Every existing ancestor is opened without following symlinks.
            // Newly needed XDG directories are created privately as well.
            directory = create_directory(&directory, name, &current_path)?;
            if Identity::of(&directory, &current_path)? == selected {
                return Err(error(
                    path,
                    ErrorKind::InvalidInput,
                    "an entry containing the home trash cannot be trashed",
                ));
            }
        }
        verify_private_directory(&directory, path)?;
        let infrastructure = |name: &str| -> Result<File, FsError> {
            let child_path = path.join(name);
            let child = create_directory(&directory, OsStr::new(name), &child_path)?;
            verify_private_directory(&child, &child_path)?;
            if Identity::of(&child, &child_path)? == selected {
                return Err(error(
                    &child_path,
                    ErrorKind::InvalidInput,
                    "trash infrastructure cannot be trashed",
                ));
            }
            Ok(child)
        };
        let files = infrastructure("files")?;
        let info = infrastructure("info")?;
        Ok(Self {
            files,
            info,
            path: path.to_owned(),
        })
    }

    fn reserve(&self, prepared: &Prepared) -> Result<Reservation, FsError> {
        loop {
            // Shorten only the internal trash label, never the native source.
            let mut bytes = prepared
                .name
                .as_bytes()
                .iter()
                .copied()
                .take(32)
                .collect::<Vec<_>>();
            bytes.extend_from_slice(format!("-{}", unique_name("midnight")).as_bytes());
            let name = OsString::from_vec(bytes);
            let mut info_name = name.clone();
            info_name.push(".trashinfo");
            let info_path = self.path.join("info").join(&info_name);
            let file = match openat(
                &self.info,
                &info_name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(file) => file,
                Err(rustix::io::Errno::EXIST) => continue,
                Err(e) => return Err(native_error(&info_path, e)),
            };
            let mut file = File::from(file);
            let reservation = Reservation {
                name,
                info_name,
                info_path: info_path.clone(),
                identity: Identity::of(&file, &info_path)?,
            };
            let metadata = format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                percent_encode(prepared.item.path.as_os_str().as_bytes()),
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
            );
            if let Err(failure) = file
                .write_all(metadata.as_bytes())
                .and_then(|()| file.sync_all())
            {
                return Err(
                    reservation.remove(self, FsError::from_io(OPERATION, &info_path, failure))
                );
            }
            return Ok(reservation);
        }
    }
}

struct Reservation {
    name: OsString,
    info_name: OsString,
    info_path: PathBuf,
    identity: Identity,
}

impl Reservation {
    fn remove(&self, trash: &Trash, mut failure: FsError) -> FsError {
        let removed = self
            .identity
            .verify(&trash.info, &self.info_name, &self.info_path)
            .and_then(|()| {
                unlinkat(&trash.info, &self.info_name, AtFlags::empty())
                    .map_err(|e| native_error(&self.info_path, e))
            });
        if let Err(cleanup) = removed {
            failure
                .message
                .push_str(&format!("; unused trash metadata remains: {cleanup}"));
        }
        failure
    }
}

fn create_directory(parent: &File, name: &OsStr, path: &Path) -> Result<File, FsError> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(e) => return Err(native_error(path, e)),
    }
    Ok(File::from(
        openat(
            parent,
            name,
            DIRECTORY_FLAGS | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|e| native_error(path, e))?,
    ))
}

fn percent_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            encoded.push(char::from(*byte));
        } else {
            write!(&mut encoded, "%{byte:02X}").expect("formatting to a string");
        }
    }
    encoded
}
