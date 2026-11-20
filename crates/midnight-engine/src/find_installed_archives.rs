//! Search inside archives through the installed archive plugins. A private provider
//! session per archive: opened from the native capture, searched, and released on
//! the same search worker. Panels and viewers never share these sessions, which count
//! in the host's allowance for searches (`Mount::open_for_search`) rather than in the
//! two archives the panels may have open.
use crate::{
    ArchiveCapture,
    find_worker::{ArchiveSearch, archive::OpenedArchive},
};
use midnight_core::*;
use midnight_filesystem_host::{Catalog, Mount};
use std::{
    io,
    path::Path,
    sync::{Arc, OnceLock},
};

pub(crate) struct InstalledArchives {
    capture: ArchiveCapture,
    /// Discovered on first use, on a search worker.
    catalog: OnceLock<Result<Catalog, String>>,
}

impl InstalledArchives {
    pub(crate) fn new(capture: ArchiveCapture) -> Self {
        Self {
            capture,
            catalog: OnceLock::new(),
        }
    }
}

fn discover() -> Result<Catalog, String> {
    let executable =
        midnight_plugin_host::installed_executable().map_err(|error| error.to_string())?;
    Catalog::discover(executable.parent().unwrap_or(Path::new(".")))
        .map_err(|error| error.to_string())
}

impl ArchiveSearch for InstalledArchives {
    fn open(
        &self,
        archive: &FindEntry,
        cancel: &Cancellation,
    ) -> Option<Result<OpenedArchive, FsError>> {
        let format = archive_format(&archive.path)?;
        let catalog = match self.catalog.get_or_init(discover) {
            Ok(catalog) => catalog,
            Err(message) => {
                return Some(Err(find_error(
                    &archive.path,
                    io::ErrorKind::Unsupported,
                    &format!("Archive plugins could not be discovered: {message}"),
                )));
            }
        };
        // No installed provider for this format: an ordinary file, not an error.
        let (package, format) = catalog.choose_format(format, None)?;
        Some((|| {
            let source = (self.capture)(&archive.path, None, cancel)?;
            let mount = Mount::open_for_search(package, format, &archive.path, source, cancel)?;
            let root = mount.address();
            let mount: Arc<dyn ArchiveMount> = mount;
            Ok(OpenedArchive { mount, root })
        })())
    }

    fn release(&self, mount: Arc<dyn ArchiveMount>) -> Result<(), FsError> {
        mount.release_on_worker()
    }
}
