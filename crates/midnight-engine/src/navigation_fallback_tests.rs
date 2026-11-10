use super::*;
use midnight_core::NavigationFallbackService;
use std::path::Path;

struct Root(PathBuf);
impl NavigationFallbackService for Root {
    fn navigation_fallback_root(&self) -> Result<PathBuf, midnight_core::FsError> {
        Ok(self.0.clone())
    }
}

#[test]
fn invalid_provider_data_cancellation_and_relative_addresses_never_become_native_recovery() {
    let root = std::env::current_dir()
        .unwrap()
        .ancestors()
        .last()
        .unwrap()
        .to_owned();
    let path = root.join("a/b");
    let service = Root(root);
    let cancel = Cancellation::default();
    for kind in [
        std::io::ErrorKind::InvalidData,
        std::io::ErrorKind::Unsupported,
        std::io::ErrorKind::Interrupted,
        std::io::ErrorKind::OutOfMemory,
    ] {
        let error = midnight_core::FsError::from_io("list", &path, kind.into());
        assert!(
            navigation_fallback::next_path(&path, &error, Some(&service), &cancel)
                .unwrap()
                .is_none()
        );
    }
    let error = midnight_core::FsError::from_io("list", &path, std::io::ErrorKind::NotFound.into());
    assert!(
        navigation_fallback::next_path(
            Path::new("archive/member"),
            &error,
            Some(&service),
            &cancel
        )
        .unwrap()
        .is_none()
    );
    cancel.cancel();
    assert!(
        navigation_fallback::next_path(&path, &error, Some(&service), &cancel)
            .unwrap()
            .is_none()
    );
}

#[test]
fn raw_long_parent_is_preserved_and_final_root_failure_has_no_next_candidate() {
    let root = std::env::current_dir()
        .unwrap()
        .ancestors()
        .last()
        .unwrap()
        .to_owned();
    let parent = root
        .join("Příliš_中文_🧪_e\u{301}".repeat(30))
        .join("parent");
    let path = parent.join("missing");
    let service = Root(root.clone());
    let cancel = Cancellation::default();
    let error = midnight_core::FsError::from_io("list", &path, std::io::ErrorKind::NotFound.into());
    assert_eq!(
        navigation_fallback::next_path(&path, &error, Some(&service), &cancel).unwrap(),
        Some(parent)
    );
    assert!(
        navigation_fallback::next_path(&root, &error, Some(&service), &cancel)
            .unwrap()
            .is_none()
    );
}
