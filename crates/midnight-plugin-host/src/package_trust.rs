//! Validate local dependency code before the native loader can resolve it.
use crate::Error;
use std::path::Path;

/// The package folder is checked with every folder above it once; each object below it is
/// then checked on its own (owner, ACL or mode, and that its path is canonical), because its
/// folder and that folder's ancestors were accepted earlier in this same walk. Repeating the
/// ancestors for every object made each video helper launch spend about a second on a
/// package with over a thousand licence notices.
pub(crate) fn check(directory: &Path) -> Result<(), Error> {
    crate::validate_plugin_path(directory)?;
    let entries = crate::trust::EntryCheck::new()?;
    let mut pending = vec![directory.to_owned()];
    let mut count = 0;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            count += 1;
            if count > 4096 {
                return Err(Error("Plugin package entry limit exceeded".into()));
            }
            let entry = entry?;
            let path = entry.path();
            // Dependency filenames need not end in .dll/.so, and ELF RUNPATH may name
            // a subdirectory. Validate every local object, not just recognized suffixes.
            entries.check(&path)?;
            if path.canonicalize()? != path {
                return Err(Error(
                    "Plugin path redirects outside its installation".into(),
                ));
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if !kind.is_file() {
                return Err(Error("Plugin package contains a nonregular object".into()));
            }
        }
    }
    Ok(())
}
