//! Creation stays below a prepared native directory without following child links.
use super::{
    changed, checked, failure, invalid, native, reject_dot_components, validate_name, validate_path,
};
use midnight_core::{Cancellation, FsError, PreparedDirectory};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
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

    fn create(&self, relative_path: &Path, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        checked(cancel, &self.path)?;
        reject_dot_components(relative_path)?;
        let components: Vec<_> = relative_path.components().collect();
        if components.is_empty() {
            return Err(invalid(relative_path, "enter a relative directory name"));
        }
        // Validate the entire request before creating even the first component.
        for component in &components {
            let Component::Normal(name) = component else {
                return Err(invalid(
                    relative_path,
                    "only relative normal path components are accepted",
                ));
            };
            validate_name(name)
                .map_err(|e| failure("validate create directory", relative_path, e))?;
        }
        self.revalidate()?;
        let mut guards: Vec<File> = Vec::new();
        let mut result = self.path.clone();
        for (index, component) in components.iter().enumerate() {
            result.push(component.as_os_str());
            checked(cancel, &result)?;
            let parent = guards.last().unwrap_or(&self.directory);
            let directory = native::create_directory(
                parent,
                component.as_os_str(),
                index + 1 == components.len(),
            )
            .map_err(|e| {
                let mut error = failure("create directory", &result, e);
                error
                    .message
                    .push_str("; any intermediate directories already created are retained");
                error
            })?;
            guards.push(directory);
        }
        Ok(result)
    }
}
