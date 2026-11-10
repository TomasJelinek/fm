//! Read-only package metadata; discovery never starts a filesystem implementation.
use midnight_core::{FilesystemPluginInfo, FilesystemPluginKind};
use midnight_native_filesystem_host::PackageKind;
use std::{collections::BTreeSet, path::Path};

pub(super) fn catalog(executable: &Path) -> Result<Vec<FilesystemPluginInfo>, String> {
    let directory = executable
        .parent()
        .ok_or("Executable directory is unavailable")?;
    let mut rows = Vec::new();
    let mut diagnostics = BTreeSet::new();
    match midnight_native_filesystem_host::Catalog::discover(directory) {
        Ok(catalog) => {
            let native_error = catalog
                .native_package()
                .err()
                .map(|error| error.to_string());
            let network_error = catalog
                .network_package()
                .err()
                .map(|error| error.to_string());
            for package in catalog.packages {
                let (kind, fallback, error) = match package.kind {
                    PackageKind::Native => (
                        FilesystemPluginKind::Native,
                        "Native filesystem",
                        &native_error,
                    ),
                    PackageKind::Network => (
                        FilesystemPluginKind::Network,
                        "Network filesystem",
                        &network_error,
                    ),
                };
                rows.push(FilesystemPluginInfo {
                    id: package.id,
                    name: package.name.unwrap_or_else(|| fallback.into()),
                    version: package.version,
                    kind: Some(kind),
                    location: Some(package.directory),
                    status: error.as_ref().map_or_else(
                        || "Installed".into(),
                        |error| format!("Unavailable: {error}"),
                    ),
                });
            }
            diagnostics.extend(catalog.diagnostics);
        }
        Err(error) => {
            diagnostics.insert(error.to_string());
        }
    }
    match midnight_filesystem_host::Catalog::discover(directory) {
        Ok(catalog) => {
            rows.extend(
                catalog
                    .packages
                    .into_iter()
                    .map(|package| FilesystemPluginInfo {
                        id: package.id,
                        name: package.name.unwrap_or_else(|| "Archive filesystem".into()),
                        version: package.version,
                        kind: Some(FilesystemPluginKind::Archive),
                        location: Some(package.directory),
                        status: "Installed".into(),
                    }),
            );
            diagnostics.extend(catalog.diagnostics);
        }
        Err(error) => {
            diagnostics.insert(error.to_string());
        }
    }
    rows.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    rows.extend(diagnostics.into_iter().map(|message| FilesystemPluginInfo {
        id: String::new(),
        name: "Unavailable filesystem package".into(),
        version: String::new(),
        kind: None,
        location: None,
        status: message,
    }));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let base = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        std::fs::write(
            root.path().join("me.exe"),
            b"catalog fixture, never executed",
        )
        .unwrap();
        root
    }

    fn network(root: &Path, directory: &str, id: &str) {
        let path = root.join("filesystems").join(directory);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("worker.dll"), b"not a library, never loaded").unwrap();
        std::fs::write(path.join("plugin.toml"), format!(
            "schema_version=1\nid='{id}'\nname='Network filesystem'\nkind='filesystem-network'\nversion='1'\nentry_library='worker.dll'\ntarget='{}'\nabi_major=1\nminimum_host_minor=2\nprotocol=1\ncapabilities=['browse','stat','volumes','activate','refresh','resolve-text']\naddress_encoding='opaque'\n",
            midnight_plugin_host::host_target()
        )).unwrap();
    }

    #[test]
    #[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
    fn fixture_catalog_lists_real_manifest_records_without_loading_code() {
        let root = fixture();
        network(root.path(), "network", "network.fixture");
        let archive = root.path().join("filesystems/archive");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(archive.join("worker.exe"), b"not executable, never started").unwrap();
        std::fs::write(archive.join("plugin.toml"), format!(
            "schema_version=1\nid='archive.fixture'\nname='Archive filesystem'\nkind='filesystem-worker'\nversion='1'\nentry_executable='worker.exe'\ntarget='{}'\nprotocol=2\nformats=['zip']\n",
            midnight_plugin_host::host_target()
        )).unwrap();
        let rows = catalog(&root.path().join("me.exe")).unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        for kind in [FilesystemPluginKind::Archive, FilesystemPluginKind::Network] {
            let row = rows.iter().find(|row| row.kind == Some(kind)).unwrap();
            assert_eq!(row.status, "Installed");
            assert_eq!(row.version, "1");
            assert!(row.location.as_ref().is_some_and(|path| path.is_absolute()));
        }
        assert_eq!(
            std::fs::read(archive.join("worker.exe")).unwrap(),
            b"not executable, never started"
        );
    }

    #[test]
    #[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
    fn fixture_catalog_retains_malformed_and_missing_artifact_diagnostics() {
        let root = fixture();
        network(root.path(), "missing-library", "network.missing");
        std::fs::remove_file(root.path().join("filesystems/missing-library/worker.dll")).unwrap();
        let malformed = root.path().join("filesystems/malformed");
        std::fs::create_dir_all(&malformed).unwrap();
        std::fs::write(malformed.join("plugin.toml"), b"not valid = [").unwrap();
        let rows = catalog(&root.path().join("me.exe")).unwrap();
        assert!(
            rows.iter()
                .all(|row| row.id.is_empty() && row.kind.is_none())
        );
        for package in ["missing-library", "malformed"] {
            assert!(
                rows.iter().any(|row| row.status.contains(package)),
                "{rows:?}"
            );
        }
    }

    #[test]
    #[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
    fn fixture_catalog_marks_ambiguous_native_provider_claims_unavailable() {
        let root = fixture();
        network(root.path(), "one", "network.one");
        network(root.path(), "two", "network.two");
        let rows = catalog(&root.path().join("me.exe")).unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(
            rows.iter()
                .all(|row| row.status.starts_with("Unavailable:"))
        );
    }

    #[test]
    #[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
    fn fixture_catalog_keeps_duplicate_provider_identity_diagnostics() {
        let root = fixture();
        network(root.path(), "one", "network.duplicate");
        network(root.path(), "two", "network.duplicate");
        let rows = catalog(&root.path().join("me.exe")).unwrap();
        assert!(rows.iter().all(|row| row.id.is_empty()));
        assert!(
            rows.iter()
                .any(|row| row.status.contains("Duplicate native filesystem ID"))
        );
    }

    #[test]
    #[ignore = "requires PLUGIN_CATALOG_TEST_EXECUTABLE in a trusted installed package"]
    fn actual_installation_lists_native_archive_and_network_without_loading_code() {
        let executable = std::path::PathBuf::from(
            std::env::var_os("PLUGIN_CATALOG_TEST_EXECUTABLE")
                .expect("PLUGIN_CATALOG_TEST_EXECUTABLE"),
        )
        .canonicalize()
        .unwrap();
        let rows = catalog(&executable).unwrap();
        assert_eq!(rows.len(), 3, "{rows:?}");
        for kind in [
            FilesystemPluginKind::Native,
            FilesystemPluginKind::Archive,
            FilesystemPluginKind::Network,
        ] {
            let row = rows.iter().find(|row| row.kind == Some(kind)).unwrap();
            assert_eq!(row.status, "Installed");
            assert!(!row.id.is_empty());
            assert!(row.location.as_ref().is_some_and(|path| path.is_absolute()));
        }
    }
}
