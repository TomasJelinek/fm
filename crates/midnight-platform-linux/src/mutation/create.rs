//! Explicit path creation using pinned directory handles. mkdirat and openat2
//! reject links at each entered component. These user-entered paths may cross
//! mounts; recursive mutation traversal retains its separate no-mount boundary.
//! Existing intermediate directories are allowed; empty files require one new basename.
//! Completed creations survive errors/cancellation.
//! Handles prevent retargeting through replaced names; as with rename/delete,
//! they cannot prevent a concurrent owner from moving an opened directory.
use super::*;

pub(super) struct Directory {
    path: PathBuf,
    directory: File,
    identity: Identity,
}

impl Directory {
    pub(super) fn prepare(path: &Path, cancel: &Cancellation) -> Result<Self, FsError> {
        check_cancel(cancel, path)?;
        let path = std::path::absolute(path).map_err(|e| FsError::from_io(OPERATION, path, e))?;
        let directory = File::from(
            openat(CWD, &path, DIRECTORY_FLAGS, Mode::empty())
                .map_err(|e| native_error(&path, e))?,
        );
        let identity = Identity::of(&directory, &path)?;
        check_cancel(cancel, &path)?;
        Ok(Self {
            path,
            directory,
            identity,
        })
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check_cancel(cancel, &self.path)?;
        let current = File::from(
            openat(CWD, &self.path, DIRECTORY_FLAGS, Mode::empty())
                .map_err(|e| native_error(&self.path, e))?,
        );
        if Identity::of(&current, &self.path)? != self.identity {
            return Err(error(
                &self.path,
                ErrorKind::InvalidInput,
                "the base directory changed",
            ));
        }
        Ok(())
    }

    fn create_components(
        &self,
        absolute: bool,
        components: &[&OsStr],
        cancel: &Cancellation,
        created: &mut bool,
    ) -> Result<PathBuf, FsError> {
        let (mut directory, mut path) = if absolute {
            (
                File::from(
                    openat(CWD, "/", DIRECTORY_FLAGS, Mode::empty())
                        .map_err(|e| native_error(Path::new("/"), e))?,
                ),
                PathBuf::from("/"),
            )
        } else {
            (
                self.directory
                    .try_clone()
                    .map_err(|e| FsError::from_io(OPERATION, &self.path, e))?,
                self.path.clone(),
            )
        };
        for (index, name) in components.iter().enumerate() {
            check_cancel(cancel, &path)?;
            if *name == OsStr::new(".") {
                continue;
            }
            path.push(name);
            if *name == OsStr::new("..") {
                // Resolve the parent of the opened directory, never a lexically
                // collapsed pathname. NO_SYMLINKS remains required; BENEATH would
                // incorrectly forbid the user's explicit request to leave this base.
                directory = File::from(
                    openat2(
                        &directory,
                        *name,
                        DIRECTORY_FLAGS | OFlags::NOFOLLOW,
                        Mode::empty(),
                        ResolveFlags::NO_SYMLINKS,
                    )
                    .map_err(|e| native_error(&path, e))?,
                );
                continue;
            }
            let final_component = index + 1 == components.len();
            match mkdirat(&directory, *name, Mode::RWXU | Mode::RWXG | Mode::RWXO) {
                Ok(()) => *created = true,
                Err(rustix::io::Errno::EXIST) if !final_component => {}
                Err(e) => return Err(native_error(&path, e)),
            }
            if !final_component {
                directory = File::from(
                    openat2(
                        &directory,
                        *name,
                        DIRECTORY_FLAGS | OFlags::NOFOLLOW,
                        Mode::empty(),
                        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
                    )
                    .map_err(|e| native_error(&path, e))?,
                );
            }
        }
        Ok(path)
    }
}

impl PreparedDirectory for Directory {
    fn create_file(&self, name: &OsStr, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        validate_name(name, &self.path)?;
        self.revalidate(cancel)?;
        let path = self.path.join(name);
        check_cancel(cancel, &path)?;
        // EXCL rejects every existing entry, including dangling links. Creating
        // a regular file writes no bytes; 0666 is reduced by the process umask.
        let _file = openat(
            &self.directory,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH,
        )
        .map_err(|e| native_error(&path, e))?;
        Ok(path)
    }

    fn create(&self, target: &Path, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        check_cancel(cancel, &self.path)?;
        // Preserve dot steps in the raw input. Path::components omits trailing '.',
        // which must be rejected before any intermediate creation takes place.
        let components = target
            .as_os_str()
            .as_bytes()
            .split(|byte| *byte == b'/')
            .filter(|component| !component.is_empty())
            .map(OsStr::from_bytes)
            .collect::<Vec<_>>();
        if components
            .last()
            .is_none_or(|name| *name == OsStr::new(".") || *name == OsStr::new(".."))
        {
            return Err(error(
                target,
                ErrorKind::InvalidInput,
                "enter a path ending in a new directory name",
            ));
        }
        for component in &components {
            if *component != OsStr::new(".") && *component != OsStr::new("..") {
                validate_name(component, target)?;
            }
        }
        self.revalidate(cancel)?;
        let mut created = false;
        self.create_components(target.is_absolute(), &components, cancel, &mut created)
            .map_err(|mut failure| {
                if created {
                    failure
                        .message
                        .push_str("; directories already created were left in place");
                }
                failure
            })
    }
}
