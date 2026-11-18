//! Immutable installation metadata prepared before a host starts viewer work.

use crate::{Catalogs, catalog_startup};
use midnight_plugin_host::Runtime;
use std::{path::Path, sync::Arc};

/// Catalogs discovered from an existing trusted application installation.
/// This grants no content authority and never loads a fabricated provider catalog.
pub struct PreparedCatalogs {
    catalogs: Catalogs,
}

impl PreparedCatalogs {
    /// Discover all viewer transports on a backend/setup worker.
    /// The supplied application and its parent must already be absolute, canonical and trusted;
    /// aliases are rejected rather than resolved into authority. Provider package, target and ABI
    /// validation remains with the ordinary catalog loaders and provider-entry checks.
    pub fn discover(application: &Path) -> Result<Self, String> {
        validate_application(application)?;
        load(application, None, None, None).map(|catalogs| Self { catalogs })
    }

    pub(crate) fn into_inner(self) -> Catalogs {
        self.catalogs
    }
}

fn validate_application(application: &Path) -> Result<(), String> {
    if !application.is_absolute() {
        return Err("Prepared catalog application path must be absolute".into());
    }
    let metadata = std::fs::symlink_metadata(application).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err("Prepared catalog application must be a regular file".into());
    }
    require_canonical(application, "application")?;
    let directory = application.parent().ok_or("Executable has no directory")?;
    require_canonical(directory, "application parent")?;
    midnight_plugin_host::validate_plugin_path(application).map_err(|error| error.to_string())?;
    midnight_plugin_host::validate_plugin_path(directory).map_err(|error| error.to_string())
}

fn require_canonical(path: &Path, description: &str) -> Result<(), String> {
    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
    // Path equality normalizes some lexical aliases; compare the supplied spelling instead.
    if canonical.as_os_str() != path.as_os_str() {
        return Err(format!(
            "Prepared catalog {description} path must already be canonical"
        ));
    }
    Ok(())
}

pub(crate) fn load(
    application: &Path,
    cached_runtime: Option<Arc<Runtime>>,
    cached_images: Option<Arc<[midnight_image_viewer_host::Provider]>>,
    cached_web: Option<Arc<[midnight_web_viewer_host::Provider]>>,
) -> Result<Catalogs, String> {
    let directory = application.parent().ok_or("Executable has no directory")?;
    catalog_startup::parallel(
        || match cached_runtime {
            Some(runtime) => Ok(runtime),
            None => Runtime::discover(directory)
                .map(Arc::new)
                .map_err(|error| error.to_string()),
        },
        || {
            Ok(cached_images.unwrap_or_else(|| {
                // Invalid process packages expose no feature. Options retains discovery diagnostics.
                midnight_image_viewer_host::discover(application)
                    .unwrap_or_default()
                    .into()
            }))
        },
        || {
            Ok(cached_web.unwrap_or_else(|| {
                midnight_web_viewer_host::discover_with_diagnostics(application)
                    .map(|discovery| discovery.providers)
                    .unwrap_or_default()
                    .into()
            }))
        },
    )
}

#[cfg(test)]
#[path = "prepared_catalogs_tests.rs"]
mod tests;
