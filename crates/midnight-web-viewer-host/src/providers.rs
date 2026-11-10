use midnight_plugin_host::{Diagnostic, validate_plugin_package, validate_plugin_path};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub version: String,
    pub extensions: Vec<String>,
    pub(crate) executable: PathBuf,
    package: PathBuf,
}

impl Provider {
    pub fn package_dir(&self) -> &Path {
        &self.package
    }
    pub(crate) fn validate(&self) -> io::Result<()> {
        validate_plugin_package(&self.package).map_err(io::Error::other)?;
        validate_plugin_path(&self.executable).map_err(io::Error::other)
    }
}

#[derive(Default)]
pub struct Discovery {
    pub providers: Vec<Provider>,
    pub diagnostics: Vec<Diagnostic>,
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

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn parse(bytes: &[u8]) -> io::Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    let mut extensions = HashSet::new();
    if manifest.schema != 1
        || manifest.protocol != 1
        || manifest.target != midnight_plugin_host::host_target()
        || !identifier(&manifest.id)
        || !identifier(&manifest.version)
        || !identifier(&manifest.executable)
        || manifest.name.is_empty()
        || manifest.name.len() > 128
        || manifest.name.chars().any(char::is_control)
        || manifest.extensions.is_empty()
        || manifest.extensions.len() > 64
        || manifest.extensions.iter().any(|value| {
            value.len() > 32
                || value.is_empty()
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                || !extensions.insert(value)
        })
    {
        return Err(io::Error::other("Invalid web-viewer manifest"));
    }
    Ok(manifest)
}

/// The application passes the already resolved private installation executable. Never search
/// PATH or the opened document directory for a browser or its native dependencies.
pub fn discover_with_diagnostics(application_executable: &Path) -> io::Result<Discovery> {
    if !application_executable.is_absolute() {
        return Err(io::Error::other("Viewer installation must be absolute"));
    }
    validate_plugin_path(application_executable).map_err(io::Error::other)?;
    let root = application_executable
        .parent()
        .ok_or_else(|| io::Error::other("Missing parent"))?
        .join("viewers");
    if !root.try_exists()? {
        return Ok(Discovery::default());
    }
    validate_plugin_path(&root).map_err(io::Error::other)?;
    let mut result = Discovery::default();
    let mut duplicates = HashSet::new();
    for (index, entry) in fs::read_dir(root)?.enumerate() {
        if index >= 128 {
            return Err(io::Error::other("Viewer package count exceeded"));
        }
        let path = entry?.path();
        let candidate = (|| {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Ok(None);
            }
            let manifest = path.join("web-viewer.json");
            if !manifest.try_exists()? {
                return Ok(None);
            }
            validate_plugin_package(&path).map_err(io::Error::other)?;
            if path.join("plugin.toml").try_exists()?
                || path.join("image-viewer.json").try_exists()?
            {
                return Err(io::Error::other("Ambiguous viewer package transports"));
            }
            let mut bytes = Vec::new();
            fs::File::open(manifest)?
                .take(16 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 16 * 1024 {
                return Err(io::Error::other("Viewer manifest too large"));
            }
            let manifest = parse(&bytes)?;
            let executable = path.join(&manifest.executable);
            if !fs::symlink_metadata(&executable)?.is_file() {
                return Err(io::Error::other(
                    "Web viewer executable is not a regular file",
                ));
            }
            Ok(Some(Provider {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                extensions: manifest.extensions,
                executable,
                package: path.clone(),
            }))
        })();
        match candidate {
            Ok(Some(provider)) => {
                if result.providers.iter().any(|item| item.id == provider.id) {
                    duplicates.insert(provider.id.clone());
                    result.diagnostics.push(Diagnostic {
                        package: path,
                        message: "Duplicate web-viewer ID".into(),
                    });
                }
                result.providers.push(provider);
            }
            Ok(None) => {}
            Err(error) => result.diagnostics.push(Diagnostic {
                package: path,
                message: error.to_string(),
            }),
        }
    }
    result
        .providers
        .retain(|item| !duplicates.contains(&item.id));
    result.providers.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifests_cannot_select_arbitrary_programs_or_other_transports() {
        let original = serde_json::json!({"schema":1,"protocol":1,"id":"midnight.viewer.html",
            "name":"HTML","version":"0.1.0","target":midnight_plugin_host::host_target(),
            "executable":"midnight-viewer-html.exe","extensions":["html","htm","xhtml"]});
        assert!(parse(&serde_json::to_vec(&original).unwrap()).is_ok());
        for (key, value) in [
            ("executable", serde_json::json!("../browser.exe")),
            ("protocol", serde_json::json!(2)),
            ("unknown", serde_json::json!(true)),
            ("extensions", serde_json::json!(["html", "html"])),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            assert!(parse(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
    }
}
