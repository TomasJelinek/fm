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
        Self(root.canonicalize().unwrap())
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
    let executable = root.0.join("me.exe");
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
