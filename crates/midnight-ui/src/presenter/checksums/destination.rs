//! Convert a manifest editor draft without treating lossy display text as a native identity.
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "destination_tests.rs"]
mod tests;

pub(crate) fn resolve(suggested: &Path, edited: &str) -> Result<PathBuf, &'static str> {
    let destination = if suggested.to_string_lossy() == edited {
        suggested.to_owned()
    } else {
        let path = PathBuf::from(edited);
        if let Some(parent) = suggested.parent() {
            if path.file_name() == Some(path.as_os_str()) {
                parent.join(path)
            } else if path.is_absolute() && crate::native_path_edit::ambiguous_edit(parent, edited)
            {
                return Err(concat!(
                    "The displayed folder contains characters this editor cannot represent. ",
                    "Enter only a new filename to keep the original folder, or choose another ",
                    "complete path without replacement characters."
                ));
            } else {
                path
            }
        } else {
            path
        }
    };
    if !destination.is_absolute() || destination.as_os_str().as_encoded_bytes().contains(&0) {
        return Err("Enter a new filename or the absolute path of a new manifest file.");
    }
    Ok(destination)
}
