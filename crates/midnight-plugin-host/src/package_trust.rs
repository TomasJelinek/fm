//! Validate local dependency code before the native loader can resolve it.
use crate::Error;
use std::path::Path;

pub(crate) fn check(directory: &Path) -> Result<(), Error> {
    let mut pending = vec![directory.to_owned()];
    let mut count = 0;
    while let Some(directory) = pending.pop() {
        crate::validate_plugin_path(&directory)?;
        for entry in std::fs::read_dir(directory)? {
            count += 1;
            if count > 4096 {
                return Err(Error("Viewer package entry limit exceeded".into()));
            }
            let entry = entry?;
            let path = entry.path();
            // Dependency filenames need not end in .dll/.so, and ELF RUNPATH may name
            // a subdirectory. Validate every local object, not just recognized suffixes.
            crate::validate_plugin_path(&path)?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if !kind.is_file() {
                return Err(Error("Viewer package contains a nonregular object".into()));
            }
        }
    }
    Ok(())
}
