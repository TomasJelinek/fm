//! Installed, trusted executable providers; media never chooses the executable or its arguments.
use midnight_core::ThumbnailerAssociations;
use serde::Deserialize;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub protocol: u32,
    pub id: String,
    pub version: String,
    pub executable: String,
    pub extensions: Vec<String>,
    pub video: bool,
}
#[derive(Clone)]
pub(crate) struct Provider {
    pub manifest: Manifest,
    pub executable: PathBuf,
}

pub(crate) fn discover(executable: &Path) -> io::Result<Vec<Provider>> {
    let executable = midnight_plugin_host::installation_path(executable)?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("Executable has no parent"))?
        .join("thumbnailers");
    if !directory.exists() {
        return Ok(vec![]);
    }
    midnight_plugin_host::validate_plugin_path(&directory).map_err(io::Error::other)?;
    let mut providers = Vec::new();
    for (index, entry) in fs::read_dir(&directory)?.enumerate() {
        if index >= 128 {
            return Err(io::Error::other(
                "Thumbnail installation entry limit exceeded",
            ));
        }
        let path = entry?.path();
        // Native decoders load only installed code beside the helper. Every candidate runtime
        // library must meet the same write/traversal trust policy as the helper executable.
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(runtime_library)
        {
            midnight_plugin_host::validate_plugin_path(&path).map_err(io::Error::other)?;
        }
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        midnight_plugin_host::validate_plugin_path(&path).map_err(io::Error::other)?;
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 {
            return Err(io::Error::other("Provider manifest exceeds limit"));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        if manifest.protocol != 1
            || !identifier(&manifest.id, 64)
            || !identifier(&manifest.version, 128)
            || !identifier(&manifest.executable, 128)
            || manifest.extensions.is_empty()
            || manifest.extensions.len() > 32
            || manifest
                .extensions
                .iter()
                .any(|extension| !identifier(extension, 16) || extension.contains('.'))
        {
            return Err(io::Error::other("Invalid thumbnail provider manifest"));
        }
        if providers
            .iter()
            .any(|provider: &Provider| provider.manifest.id == manifest.id)
        {
            return Err(io::Error::other("Duplicate thumbnail provider identifier"));
        }
        let executable = directory.join(&manifest.executable);
        midnight_plugin_host::validate_plugin_path(&executable).map_err(io::Error::other)?;
        providers.push(Provider {
            manifest,
            executable,
        });
    }
    providers.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    Ok(providers)
}

/// Providers are the trusted catalog in provider-ID order.
pub(crate) fn select<'a>(
    providers: &'a [Provider],
    associations: &ThumbnailerAssociations,
    extension: &str,
    image_only: bool,
) -> Option<&'a Provider> {
    providers.iter().find(|provider| {
        (!image_only || !provider.manifest.video)
            && associations
                .effective_extensions(&provider.manifest.id, &provider.manifest.extensions)
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    })
}

fn runtime_library(name: &str) -> bool {
    name.get(name.len().saturating_sub(4)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".dll"))
        || name.ends_with(".so")
        || name.contains(".so.")
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
    use midnight_core::ThumbnailerAssociations;

    fn provider(id: &str, extensions: &[&str], video: bool) -> Provider {
        Provider {
            manifest: Manifest {
                protocol: 1,
                id: id.into(),
                version: "1".into(),
                executable: "worker".into(),
                extensions: extensions
                    .iter()
                    .map(|extension| (*extension).into())
                    .collect(),
                video,
            },
            executable: PathBuf::from("worker"),
        }
    }

    #[test]
    fn thumbnailer_provider_resolution_observes_overrides_and_catalog_precedence() {
        let providers = [
            provider("a.image", &["jpg"], false),
            provider("b.image", &["jpg"], false),
        ];
        let mut choices = ThumbnailerAssociations::default();
        let pick = |choices: &ThumbnailerAssociations| {
            select(&providers, choices, "JpG", false).map(|provider| provider.manifest.id.as_str())
        };
        assert_eq!(pick(&choices), Some("a.image"));
        choices
            .overrides
            .insert("a.image".into(), vec!["bmp".into()]);
        assert_eq!(pick(&choices), Some("b.image"));
        choices.overrides.insert("b.image".into(), Vec::new());
        assert_eq!(pick(&choices), None);
        assert!(choices.reset("a.image"));
        assert_eq!(pick(&choices), Some("a.image"));
    }

    #[test]
    fn artwork_excludes_video_before_matching_the_effective_extension() {
        let providers = [
            provider("a.video", &["jpg"], true),
            provider("b.image", &["png"], false),
        ];
        let choices = ThumbnailerAssociations {
            overrides: [("b.image".into(), vec!["JPG".into()])].into(),
        };
        assert_eq!(
            select(&providers, &choices, "jpg", false)
                .unwrap()
                .manifest
                .id,
            "a.video"
        );
        assert_eq!(
            select(&providers, &choices, "jpg", true)
                .unwrap()
                .manifest
                .id,
            "b.image"
        );
        assert!(select(&providers, &choices, "png", true).is_none());
    }

    #[test]
    fn windows_loader_case_variants_all_require_library_trust_validation() {
        for name in [
            "avcodec-63.dll",
            "AVCODEC-63.DLL",
            "avcodec-63.DlL",
            "libavcodec.so.63",
        ] {
            assert!(super::runtime_library(name));
        }
        assert!(!super::runtime_library("notices"));
    }
}
