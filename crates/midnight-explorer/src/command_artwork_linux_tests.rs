use super::selected_image_target;
use std::{
    fs, io,
    os::unix::fs::symlink,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/tmp")
            .join(format!(
                "command-artwork-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn selected_theme_icon_resolves_relative_link_chains_to_the_regular_image() {
    let fixture = Fixture::new();
    let target = fixture.0.join("image 文档");
    fs::write(&target, b"selected artwork").unwrap();
    symlink("image 文档", fixture.0.join("edit-select-all.png")).unwrap();
    let selected = fixture.0.join("gtk-select-all.png");
    symlink("edit-select-all.png", &selected).unwrap();

    let resolved = selected_image_target(&selected).unwrap();
    assert_eq!(resolved, target);
    assert!(fs::symlink_metadata(&resolved).unwrap().is_file());
    assert_eq!(fs::read(&resolved).unwrap(), b"selected artwork");
}

#[test]
fn unavailable_selected_icon_targets_fail_resolution() {
    let fixture = Fixture::new();
    let dangling = fixture.0.join("dangling.png");
    symlink("missing.png", &dangling).unwrap();
    assert_eq!(
        selected_image_target(&dangling).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );

    let cyclic = fixture.0.join("cyclic.png");
    symlink("cyclic.png", &cyclic).unwrap();
    assert!(selected_image_target(&cyclic).is_err());
}
