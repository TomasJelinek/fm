//! Public discovery must classify transports without starting installed code.
use super::*;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let base = std::env::var_os("HOST_TEST_INSTALL_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "fm-image-catalog-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root.canonicalize().unwrap());
        crate::require_private_fixture_root(&fixture.0);
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // This fresh private fixture contains only the files written below.
        assert_eq!(self.0.canonicalize().unwrap(), self.0);
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn image_and_web_manifest_pair_is_rejected_without_loading_a_provider() {
    let root = Fixture::new();
    let executable = root.0.join("midnight-explorer.exe");
    fs::write(&executable, b"discovery fixture, never executed").unwrap();
    let package = root.0.join("viewers/image");
    fs::create_dir_all(&package).unwrap();
    let manifest = serde_json::json!({
        "schema": 1, "protocol": 1, "id": "image.fixture", "name": "Image viewer",
        "version": "1", "target": midnight_plugin_host::host_target(),
        "executable": "worker.exe", "extensions": ["png"]
    });
    fs::write(
        package.join("image-viewer.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        package.join("worker.exe"),
        b"not an executable; never loaded",
    )
    .unwrap();
    fs::write(package.join("web-viewer.json"), b"{}").unwrap();
    let result = discover_with_diagnostics(&executable).unwrap();
    assert!(result.providers.is_empty(), "ambiguous bundle was admitted");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].message.contains("Ambiguous"));
}

/// A package fixture with nested licence notices, as installed image and PDF viewers carry.
fn notices_package(root: &Fixture) -> (PathBuf, PathBuf) {
    let executable = root.0.join("midnight-explorer.exe");
    fs::write(&executable, b"discovery fixture, never executed").unwrap();
    let package = root.0.join("viewers/image");
    fs::create_dir_all(&package).unwrap();
    let manifest = serde_json::json!({
        "schema": 1, "protocol": 1, "id": "image.fixture", "name": "Image viewer",
        "version": "1", "target": midnight_plugin_host::host_target(),
        "executable": "worker.exe", "extensions": ["png"]
    });
    fs::write(
        package.join("image-viewer.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        package.join("worker.exe"),
        b"not an executable; never loaded",
    )
    .unwrap();
    for crate_name in ["first-1.0.0", "second-2.0.0", "third-3.0.0"] {
        let notices = package.join("notices").join(crate_name);
        fs::create_dir_all(&notices).unwrap();
        for file in ["LICENSE-MIT", "LICENSE-APACHE", "COPYING"] {
            fs::write(notices.join(file), b"licence text").unwrap();
        }
    }
    (executable, package)
}

#[test]
fn nested_package_objects_are_validated_once_each_and_admitted() {
    let root = Fixture::new();
    let (executable, package) = notices_package(&root);
    let result = discover_with_diagnostics(&executable).unwrap();
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.providers.len(), 1);
    assert_eq!(result.providers[0].package_dir(), package);
    // Every opening revalidates the installed package with the same walk.
    result.providers[0].validate().unwrap();
}

#[test]
fn a_redirected_object_deep_in_the_package_is_refused_at_discovery_and_validation() {
    let root = Fixture::new();
    let (executable, package) = notices_package(&root);
    let provider = discover_with_diagnostics(&executable)
        .unwrap()
        .providers
        .pop()
        .unwrap();
    let outside = root.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let link = package.join("notices/second-2.0.0/redirect");
    #[cfg(windows)]
    {
        // A junction needs no privilege, unlike a directory symbolic link.
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let result = discover_with_diagnostics(&executable).unwrap();
    assert!(result.providers.is_empty(), "a redirected package was admitted");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(
        provider.validate().is_err(),
        "an installed package that gained a redirection stays valid"
    );
    // Remove the redirection itself, never what it points to.
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
}

#[test]
fn a_package_deeper_than_its_budget_is_refused() {
    let root = Fixture::new();
    let (executable, package) = notices_package(&root);
    let mut deep = package.join("notices");
    for level in 0..8 {
        deep = deep.join(format!("level-{level}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("LICENSE"), b"too deep").unwrap();
    let result = discover_with_diagnostics(&executable).unwrap();
    assert!(result.providers.is_empty());
    assert!(
        result.diagnostics[0].message.contains("budget"),
        "{:?}",
        result.diagnostics
    );
}
