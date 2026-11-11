//! Presence-only folder listing. No child reference or expansion authority escapes.
use super::{OpenedLocation, ProviderError};
use midnight_core::{Cancellation, EntryKind};

pub(crate) fn probe(opened: &OpenedLocation, cancel: &Cancellation) -> Result<bool, ProviderError> {
    super::registry::check_cancel(cancel)?;
    let mut present = false;
    let mut failure = None;
    opened.mount.list(&opened.root, cancel, &mut |entry| {
        if present || failure.is_some() {
            return false;
        }
        if cancel.is_cancelled() {
            failure = Some(ProviderError::Cancelled);
            return false;
        }
        if entry.token == opened.root {
            failure = Some(ProviderError::InvalidResponse(
                "self-referencing folder probe entry",
            ));
            return false;
        }
        present = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
        !present
    })?;
    super::registry::check_cancel(cancel)?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(present)
}
