//! Creation relative to pinned directory handles. mkdirat and openat2 enforce
//! the no-link/no-mount boundary for each entered directory component. Existing
//! intermediate directories are allowed; empty files require one new basename.
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
        components: &[&OsStr],
        cancel: &Cancellation,
        created: &mut bool,
    ) -> Result<PathBuf, FsError> {
        let mut directory = self
            .directory
            .try_clone()
            .map_err(|e| FsError::from_io(OPERATION, &self.path, e))?;
        let mut path = self.path.clone();
        for (index, name) in components.iter().enumerate() {
            check_cancel(cancel, &path)?;
            path.push(name);
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
                        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
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

    fn create(&self, relative_path: &Path, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        // Validate the complete raw input before the first mkdir. Path::components
        // alone normalizes embedded dot/empty segments, which we must reject.
        let components = relative_path
            .as_os_str()
            .as_bytes()
            .split(|byte| *byte == b'/')
            .map(OsStr::from_bytes)
            .collect::<Vec<_>>();
        for component in &components {
            validate_name(component, relative_path)?;
        }
        self.revalidate(cancel)?;
        let mut created = false;
        self.create_components(&components, cancel, &mut created)
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
