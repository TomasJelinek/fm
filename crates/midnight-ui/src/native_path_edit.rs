//! Pure guard for edited path text whose displayed native directory lost information.
use std::path::Path;

/// Preserve untouched native identities before calling. File fields pass their
/// original parent; directory fields pass the original directory itself.
pub(crate) fn ambiguous_edit(directory: &Path, edited: &str) -> bool {
    directory.to_str().is_none() && edited.contains('\u{fffd}')
}

#[cfg(test)]
#[path = "native_path_edit_tests.rs"]
mod tests;
