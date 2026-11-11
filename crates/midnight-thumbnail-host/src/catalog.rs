//! Read-only metadata using exactly the discovery policy used by thumbnail workers.
use crate::providers::{self, Provider};
use midnight_core::ThumbnailerPluginInfo;
use std::{io, path::Path};

/// Discover installed thumbnail helpers without starting workers or touching the cache.
///
/// An absent thumbnailers directory produces an empty catalog. Any discovery or trust
/// failure returns an error, matching the thumbnail service's all-or-nothing policy.
/// Call this on a backend worker because discovery reads installation metadata.
pub fn catalog(executable: &Path) -> io::Result<Vec<ThumbnailerPluginInfo>> {
    providers::discover(executable).map(|providers| providers.into_iter().map(metadata).collect())
}

fn metadata(provider: Provider) -> ThumbnailerPluginInfo {
    let manifest = provider.manifest;
    let name = match manifest.id.as_str() {
        "midnight-image" => "Image thumbnailer",
        "midnight-video" => "Video thumbnailer",
        id => id,
    }
    .to_owned();
    ThumbnailerPluginInfo {
        id: manifest.id,
        name,
        version: manifest.version,
        extensions: manifest.extensions,
        video: manifest.video,
        location: provider.executable,
        status: "Installed".into(),
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
