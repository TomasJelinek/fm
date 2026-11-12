//! Presence hints do not issue child identities or retain irrelevant file rows.
use super::{NativeProvider, ProviderError, check_cancel, native_location};
use midnight_core::{Cancellation, EntryKind};
use std::path::Path;

impl NativeProvider {
    pub(crate) fn has_subfolders(
        &self,
        directory: &Path,
        cancel: &Cancellation,
    ) -> Result<bool, ProviderError> {
        check_cancel(cancel)?;
        native_location(directory)?;
        let mut present = false;
        let mut failure = None;
        let result = self
            .filesystem
            .read_directory(directory, cancel, &mut |entry| {
                if present || failure.is_some() {
                    return false;
                }
                if cancel.is_cancelled() {
                    failure = Some(ProviderError::Cancelled);
                    return false;
                }
                let valid = entry.native_path().is_some_and(|path| {
                    path.is_absolute()
                        && path.parent() == Some(directory)
                        && path.file_name().is_some_and(|name| name == entry.name)
                        && !path.as_os_str().as_encoded_bytes().contains(&0)
                });
                if !valid {
                    failure = Some(ProviderError::InvalidResponse(
                        "native presence entry outside its directory",
                    ));
                    return false;
                }
                present = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
                !present
            });
        check_cancel(cancel)?;
        if let Some(error) = failure {
            return Err(error);
        }
        result.map_err(|error| {
            ProviderError::Failed(format!(
                "{error}\nNative path: {:?}\nError kind: {:?}",
                error.path, error.kind,
            ))
        })?;
        Ok(present)
    }
}

#[cfg(test)]
#[path = "native_presence_tests.rs"]
mod tests;
