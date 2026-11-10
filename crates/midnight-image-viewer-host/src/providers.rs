//! Trusted installed process packages; opened content cannot select executable code.
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_PACKAGES: usize = 128;
const MAX_PACKAGE_ENTRIES: usize = 512;

/// A provider discovered in a validated installation, never from a source path or search path.
#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub version: String,
    pub extensions: Vec<String>,
    pub(crate) protocol: u32,
    pub(crate) executable: PathBuf,
    package: PathBuf,
}

impl Provider {
    /// Paged document workers cannot be treated as a complete single-image input.
    pub fn is_document(&self) -> bool {
        self.protocol == 2
    }

    /// Trusted installation directory for diagnostics and the Options list.
    pub fn package_dir(&self) -> &Path {
        &self.package
    }

    /// Match a filename extension without a leading dot. Native source identities stay separate.
    pub fn supports_extension(&self, extension: &str) -> bool {
        self.extensions
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        validate_package(&self.package)?;
        midnight_plugin_host::validate_plugin_path(&self.executable).map_err(io::Error::other)
    }
}

/// Valid providers and package-scoped diagnostics for the Options list.
#[derive(Default)]
pub struct Discovery {
    pub providers: Vec<Provider>,
    pub diagnostics: Vec<midnight_plugin_host::Diagnostic>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    protocol: u32,
    id: String,
    name: String,
    version: String,
    target: String,
    executable: String,
    extensions: Vec<String>,
}

fn parse_manifest(bytes: &[u8]) -> io::Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    if manifest.schema != 1
        || !matches!(manifest.protocol, 1 | 2)
        || manifest.target != midnight_plugin_host::host_target()
        || !identifier(&manifest.id, 64)
        || !identifier(&manifest.version, 64)
        || !identifier(&manifest.executable, 128)
        || manifest.name.is_empty()
        || manifest.name.len() > 128
        || manifest.name.chars().any(char::is_control)
        || manifest.extensions.is_empty()
        || manifest.extensions.len() > 64
        || manifest.extensions.iter().any(|extension| {
            extension.is_empty()
                || extension.len() > 32
                || !extension
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid image-viewer manifest",
        ));
    }
    let mut extensions = HashSet::new();
    if manifest
        .extensions
        .iter()
        .any(|extension| !extensions.insert(extension.to_ascii_lowercase()))
    {
        return Err(io::Error::other("Duplicate image-viewer extension"));
    }
    Ok(manifest)
}

/// Discover providers relative to the full application executable path (not its directory).
/// Invalid individual packages are excluded; use `discover_with_diagnostics` to explain them.
pub fn discover(application_executable: &Path) -> io::Result<Vec<Provider>> {
    Ok(discover_with_diagnostics(application_executable)?.providers)
}

/// Read bounded manifests from `<application directory>/viewers/<package>/image-viewer.json`.
/// Package trees and every ancestor are permission checked before any installed code is started.
pub fn discover_with_diagnostics(application_executable: &Path) -> io::Result<Discovery> {
    let executable = crate::paths::installation_path(application_executable)?;
    let root = executable
        .parent()
        .ok_or_else(|| io::Error::other("Application has no parent"))?
        .join("viewers");
    if !root.try_exists()? {
        return Ok(Discovery::default());
    }
    midnight_plugin_host::validate_plugin_path(&root).map_err(io::Error::other)?;
    let mut result = Discovery::default();
    let mut duplicate_ids = HashSet::new();
    for (index, entry) in fs::read_dir(&root)?.enumerate() {
        if index >= MAX_PACKAGES {
            return Err(io::Error::other("Viewer package count exceeded"));
        }
        let path = entry?.path();
        let candidate = (|| {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Ok(None);
            }
            midnight_plugin_host::validate_plugin_path(&path).map_err(io::Error::other)?;
            let manifest_path = path.join("image-viewer.json");
            if !manifest_path.try_exists()? {
                return Ok(None);
            }
            validate_package(&path)?;
            if path.join("plugin.toml").try_exists()?
                || path.join("web-viewer.json").try_exists()?
            {
                return Err(io::Error::other(
                    "Ambiguous native, image or web viewer package",
                ));
            }
            let mut bytes = Vec::new();
            fs::File::open(&manifest_path)?
                .take(16 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 16 * 1024 {
                return Err(io::Error::other("Image-viewer manifest exceeds budget"));
            }
            let manifest = parse_manifest(&bytes)?;
            let executable = path.join(&manifest.executable);
            if !fs::symlink_metadata(&executable)?.is_file() {
                return Err(io::Error::other(
                    "Image-viewer executable is not a regular file",
                ));
            }
            midnight_plugin_host::validate_plugin_path(&executable).map_err(io::Error::other)?;
            Ok(Some(Provider {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                extensions: manifest.extensions,
                protocol: manifest.protocol,
                executable,
                package: path.clone(),
            }))
        })();
        match candidate {
            Ok(Some(provider)) => {
                if result
                    .providers
                    .iter()
                    .any(|previous| previous.id == provider.id)
                {
                    duplicate_ids.insert(provider.id.clone());
                    result.diagnostics.push(midnight_plugin_host::Diagnostic {
                        package: path,
                        message: "Duplicate image-viewer identifier".into(),
                    });
                }
                result.providers.push(provider);
            }
            Ok(None) => {}
            Err(error) => result.diagnostics.push(midnight_plugin_host::Diagnostic {
                package: path,
                message: error.to_string(),
            }),
        }
    }
    result
        .providers
        .retain(|provider| !duplicate_ids.contains(&provider.id));
    result
        .providers
        .sort_by(|left, right| left.id.cmp(&right.id));
    Ok(result)
}

fn validate_package(package: &Path) -> io::Result<()> {
    let mut pending = vec![(package.to_path_buf(), 0)];
    let mut count = 0;
    while let Some((path, depth)) = pending.pop() {
        count += 1;
        if count > MAX_PACKAGE_ENTRIES || depth > 8 {
            return Err(io::Error::other(
                "Image-viewer package traversal budget exceeded",
            ));
        }
        midnight_plugin_host::validate_plugin_path(&path).map_err(io::Error::other)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(io::Error::other(
                "Image-viewer package contains redirected or special files",
            ));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                if pending.len() + count >= MAX_PACKAGE_ENTRIES {
                    return Err(io::Error::other(
                        "Image-viewer package entry budget exceeded",
                    ));
                }
                pending.push((entry?.path(), depth + 1));
            }
        }
    }
    Ok(())
}

fn identifier(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> serde_json::Value {
        serde_json::json!({ "schema": 1, "protocol": 1, "id": "midnight.viewer.image", "name": "Images",
            "version": "0.1.0", "target": midnight_plugin_host::host_target(), "executable": "image.exe",
            "extensions": ["png", "jpg"] })
    }

    #[test]
    fn strict_manifest_rejects_unknown_fields_paths_targets_and_duplicate_extensions() {
        assert!(parse_manifest(&serde_json::to_vec(&manifest()).unwrap()).is_ok());
        for (field, invalid) in [
            ("unknown", serde_json::json!(1)),
            ("schema", serde_json::json!(2)),
            ("target", serde_json::json!("foreign")),
            ("executable", serde_json::json!("../image.exe")),
            ("extensions", serde_json::json!(["png", "PNG"])),
            ("extensions", serde_json::json!(["png;exe"])),
            ("name", serde_json::json!("a\nb")),
        ] {
            let mut value = manifest();
            value[field] = invalid;
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn missing_installation_has_no_image_feature() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/image-viewer-nonexistent-installation");
        assert!(discover(&root.join("me.exe")).unwrap().is_empty());
    }

    #[test]
    fn document_protocol_is_supported_without_accepting_future_versions() {
        let mut value = manifest();
        value["protocol"] = 2.into();
        value["extensions"] = serde_json::json!(["pdf"]);
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_ok());
        for unsupported in [0, 3, u32::MAX] {
            value["protocol"] = unsupported.into();
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }
}

#[cfg(test)]
#[path = "providers/catalog_tests.rs"]
mod catalog_tests;
