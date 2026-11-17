//! Steps shared by the viewer (`midnight-api-package`) and worker (`worker-packages`) bundles.
use crate::REGISTRY;
use flate2::read::GzDecoder;
use std::{
    ffi::OsStr,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Run one Cargo packaging command from the repository root. Its target directory
/// stays inside the fresh bundle, so concurrent bundles cannot share artifacts.
pub fn run_cargo<I, S>(cargo: &OsStr, root: &Path, output: &Path, arguments: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(cargo)
        .current_dir(root)
        .args(arguments)
        .env("CARGO_TARGET_DIR", output.join("cargo"))
        .status()?;
    if !status.success() {
        return Err(format!("Cargo packaging command failed: {status}").into());
    }
    Ok(())
}

/// Bundle-relative Cargo configuration: `midnight` registry packages resolve from the
/// bundle's local registry `source`; `vendor` also redirects crates.io to `vendor/`.
pub fn bundle_configuration(source: &str, vendor: bool) -> String {
    let mut config = format!(
        concat!(
            "[registries.midnight]\nindex = {registry:?}\n\n",
            "[source.midnight]\nregistry = {registry:?}\n",
            "replace-with = '{source}'\n\n",
            "[source.{source}]\nlocal-registry = 'registry'\n",
        ),
        registry = REGISTRY,
        source = source,
    );
    if vendor {
        config.push_str(concat!(
            "\n[source.crates-io]\nreplace-with = 'third-party'\n\n",
            "[source.third-party]\ndirectory = 'vendor'\n",
        ));
    }
    config
}

/// The `.crate` archive Cargo wrote for package `name` into the bundle's target directory.
pub fn packaged_crate(output: &Path, name: &str) -> Result<PathBuf> {
    let prefix = format!("{name}-");
    fs::read_dir(output.join("cargo/package"))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|file| file.starts_with(&prefix) && file.ends_with(".crate"))
        })
        .ok_or_else(|| format!("Cargo did not produce the expected package {name}").into())
}

/// One bounded (at most 1 MiB) regular-file member of a `.crate` archive.
pub fn crate_member(archive: &[u8], name: &str) -> Result<Vec<u8>> {
    const LIMIT: u64 = 1024 * 1024;
    let mut tar = tar::Archive::new(GzDecoder::new(Cursor::new(archive)));
    for entry in tar.entries()? {
        let entry = entry?;
        if entry.path()?.as_ref() != Path::new(name) {
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() > LIMIT {
            return Err(format!("invalid release member {name}").into());
        }
        let mut bytes = Vec::new();
        entry.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(format!("oversized release member {name}").into());
        }
        return Ok(bytes);
    }
    Err(format!("missing release member {name}").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_configuration_names_the_local_registry_and_optional_vendor_directory() {
        assert_eq!(
            bundle_configuration("worker-releases", false),
            concat!(
                "[registries.midnight]\n",
                "index = \"https://midnight-explorer.invalid/api-index\"\n\n",
                "[source.midnight]\n",
                "registry = \"https://midnight-explorer.invalid/api-index\"\n",
                "replace-with = 'worker-releases'\n\n",
                "[source.worker-releases]\nlocal-registry = 'registry'\n",
            )
        );
        assert!(
            bundle_configuration("midnight-releases", true).ends_with(concat!(
                "local-registry = 'registry'\n\n",
                "[source.crates-io]\nreplace-with = 'third-party'\n\n",
                "[source.third-party]\ndirectory = 'vendor'\n",
            ))
        );
    }
}
