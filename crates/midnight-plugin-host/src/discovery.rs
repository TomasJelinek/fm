use crate::{Error, api::viewer};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_PACKAGES: usize = 256;

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: Option<String>,
    pub version: String,
    pub description: String,
    pub models: Vec<String>,
    pub modes: Vec<u16>,
    pub extensions: Vec<String>,
    pub priority: i32,
    pub package_dir: PathBuf,
    pub entry_library: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub package: PathBuf,
    pub message: String,
}

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    id: String,
    kind: String,
    version: String,
    #[serde(default)]
    name: Option<String>,
    entry_library: String,
    target: String,
    abi_major: u16,
    minimum_host_minor: u16,
    #[serde(default)]
    required_host_services: Vec<String>,
    models: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    modes: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    priority: i32,
}

pub fn host_target() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "unsupported"
    }
}

pub(crate) fn discover(
    executable_directory: &Path,
) -> Result<(Vec<PluginInfo>, Vec<Diagnostic>), Error> {
    let base = executable_directory.canonicalize()?;
    let root = base.join("viewers");
    if !root.try_exists()? {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut diagnostics = Vec::new();
    if let Err(error) = crate::trust::check(&base).and_then(|()| crate::trust::check(&root)) {
        return Ok((
            Vec::new(),
            vec![Diagnostic {
                package: root,
                message: error.0,
            }],
        ));
    }
    if root.canonicalize()? != root {
        return Ok((
            Vec::new(),
            vec![Diagnostic {
                package: root,
                message: "Viewer root escapes installation".into(),
            }],
        ));
    }
    let mut catalog = Vec::new();
    for (index, entry) in fs::read_dir(&root)?.enumerate() {
        if index >= MAX_PACKAGES {
            diagnostics.push(Diagnostic {
                package: root.clone(),
                message: "Package count limit".into(),
            });
            break;
        }
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match native_package(&path) {
            Ok(Some(info)) => catalog.push(info),
            Ok(None) => {}
            Err(error) => diagnostics.push(Diagnostic {
                package: path,
                message: error.0,
            }),
        }
    }
    let mut counts = HashMap::new();
    for info in &catalog {
        *counts.entry(info.id.clone()).or_insert(0usize) += 1;
    }
    catalog.retain(|info| {
        if counts[&info.id] == 1 {
            return true;
        }
        diagnostics.push(Diagnostic {
            package: info.package_dir.clone(),
            message: format!("Duplicate plugin ID: {}", info.id),
        });
        false
    });
    catalog.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((catalog, diagnostics))
}

fn require(valid: bool, message: &str) -> Result<(), Error> {
    if valid {
        Ok(())
    } else {
        Err(Error(message.into()))
    }
}

fn native_package(path: &Path) -> Result<Option<PluginInfo>, Error> {
    let image = path.join("image-viewer.json").try_exists()?;
    let web = path.join("web-viewer.json").try_exists()?;
    if image || web {
        require(
            !(image && web) && !path.join("plugin.toml").try_exists()?,
            "Ambiguous viewer package: both native and worker manifests, or multiple worker manifests",
        )?;
        // Its manifest, code and dependencies are validated by the process viewer host.
        return Ok(None);
    }
    package(path).map(Some)
}

fn package(path: &Path) -> Result<PluginInfo, Error> {
    crate::trust::check(path)?;
    require(
        path.canonicalize()? == path,
        "Package path redirects outside its directory",
    )?;
    let manifest_path = path.join("plugin.toml");
    crate::trust::check(&manifest_path)?;
    require(
        manifest_path.canonicalize()? == manifest_path,
        "Manifest path is redirected",
    )?;
    let file = crate::native::open_read(&manifest_path)?;
    require(file.metadata()?.is_file(), "Manifest is not a regular file")?;
    let mut source = String::new();
    file.take(MAX_MANIFEST + 1).read_to_string(&mut source)?;
    require(
        source.len() as u64 <= MAX_MANIFEST,
        "Manifest exceeds byte limit",
    )?;
    let manifest: Manifest = toml::from_str(&source).map_err(|error| Error(error.to_string()))?;
    require(
        manifest.schema_version == 1 && manifest.kind == "viewer",
        "Unsupported manifest schema or kind",
    )?;
    #[allow(clippy::absurd_extreme_comparisons)]
    // ABI minor zero still uses the forward-compatible inequality.
    require(
        manifest.abi_major == crate::api::abi::ABI_MAJOR
            && manifest.minimum_host_minor <= crate::api::abi::ABI_MINOR,
        "Incompatible ABI",
    )?;
    require(
        manifest.target == host_target() && host_target() != "unsupported",
        "Incompatible target",
    )?;
    require(
        !manifest.id.is_empty()
            && manifest.id.len() <= 128
            && manifest
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)),
        "Invalid plugin ID",
    )?;
    require(
        !manifest.version.is_empty()
            && manifest.version.len() <= 64
            && manifest.description.len() <= 4096
            && manifest.name.as_ref().is_none_or(|name| {
                !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
            }),
        "Invalid metadata",
    )?;
    let name = &manifest.entry_library;
    require(
        !name.is_empty()
            && name.len() <= 255
            && !matches!(name.as_str(), "." | "..")
            && !name.contains(['/', '\\', ':', '\0'])
            && !name.ends_with(['.', ' ']),
        "Entry library must be one filename",
    )?;
    require(
        manifest.required_host_services.len() <= 16
            && manifest
                .required_host_services
                .iter()
                .all(|service| matches!(service.as_str(), "content.read.v1" | "work.schedule.v1")),
        "Unsupported required host service",
    )?;
    require(
        !manifest.models.is_empty()
            && manifest.models.len() <= 3
            && manifest.models.iter().all(|model| {
                matches!(
                    model.as_str(),
                    "text-page.v1" | "binary-page.v1" | "document-blocks.v1"
                )
            }),
        "Unsupported viewer model",
    )?;
    require(
        manifest.extensions.len() <= 512
            && manifest.extensions.iter().all(|extension| {
                extension.len() <= 64
                    && extension
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-+".contains(&b))
            }),
        "Invalid extensions",
    )?;
    let modes = if manifest.modes.is_empty() {
        manifest
            .models
            .iter()
            .map(|model| match model.as_str() {
                "binary-page.v1" => viewer::MODE_BINARY,
                "document-blocks.v1" => viewer::MODE_DOCUMENT,
                _ => viewer::MODE_TEXT,
            })
            .collect()
    } else {
        require(manifest.modes.len() <= 4, "Mode count limit")?;
        manifest
            .modes
            .iter()
            .map(|mode| match mode.as_str() {
                "text" => Ok(viewer::MODE_TEXT),
                "source" => Ok(viewer::MODE_SOURCE),
                "binary" => Ok(viewer::MODE_BINARY),
                "document" => Ok(viewer::MODE_DOCUMENT),
                _ => Err(Error("Unsupported mode".into())),
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    require(
        modes.iter().all(|mode| {
            let model = match *mode {
                viewer::MODE_BINARY => "binary-page.v1",
                viewer::MODE_DOCUMENT => "document-blocks.v1",
                _ => "text-page.v1",
            };
            manifest.models.iter().any(|declared| declared == model)
        }),
        "Viewer mode requires an undeclared model",
    )?;
    let library = path.join(name);
    crate::trust::check(&library)?;
    require(
        library.canonicalize()? == library && library.is_file(),
        "Library path is redirected or not a regular file",
    )?;
    crate::package_trust::check(path)?;
    Ok(PluginInfo {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        models: manifest.models,
        modes,
        extensions: manifest.extensions,
        priority: manifest.priority.clamp(-1000, 1000),
        package_dir: path.into(),
        entry_library: library,
    })
}

pub(crate) fn choose<'a>(
    catalog: &'a [PluginInfo],
    path: &Path,
    basic: bool,
    disabled: &[String],
) -> Option<(&'a PluginInfo, u16)> {
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    catalog
        .iter()
        .filter(|info| !disabled.contains(&info.id))
        .filter_map(|info| {
            let matched = !basic
                && info
                    .extensions
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(extension));
            let fallback = info.extensions.is_empty() && info.modes.contains(&viewer::MODE_TEXT);
            if !matched && !fallback {
                return None;
            }
            let mode = if matched && info.modes.contains(&viewer::MODE_DOCUMENT) {
                viewer::MODE_DOCUMENT
            } else if matched && info.modes.contains(&viewer::MODE_SOURCE) {
                viewer::MODE_SOURCE
            } else if info.modes.contains(&viewer::MODE_TEXT) {
                viewer::MODE_TEXT
            } else {
                return None;
            };
            Some((info, mode, (matched, mode, info.priority)))
        })
        .max_by_key(|(_, _, rank)| *rank)
        .map(|(info, mode, _)| (info, mode))
}
