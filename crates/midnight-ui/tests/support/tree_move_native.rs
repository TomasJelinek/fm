//! Fresh owned native files; platform policy is isolated in the sibling helpers.
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
#[path = "tree_move_windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "tree_move_linux.rs"]
mod platform;

pub struct Fixture {
    pub root: PathBuf,
    parent: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let configured = std::env::temp_dir();
        let parent = if configured
            .components()
            .any(|part| part.as_os_str() == "_build")
        {
            // Native Linux qualification can select an owned ext4 build directory.
            configured
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp")
        };
        fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        assert!(
            parent.components().any(|part| part.as_os_str() == "_build"),
            "native test fixtures require TEMP/TMPDIR below _build"
        );
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = parent.join(format!(
            "tree-move-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root, parent };
        platform::secure(&fixture.root);
        for name in ["large", "small", "empty"] {
            fs::create_dir_all(fixture.root.join("folder").join(name)).unwrap();
        }
        fs::create_dir_all(fixture.root.join("folder/large/nested/empty")).unwrap();
        fs::create_dir(fixture.root.join("destination")).unwrap();
        fs::write(
            fixture.root.join("folder/large/payload.bin"),
            vec![42; 1000],
        )
        .unwrap();
        fs::write(fixture.root.join("folder/small/payload.bin"), vec![43; 12]).unwrap();
        fs::write(fixture.root.join("root.txt"), b"unselected file").unwrap();
        fixture
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Both exact boundaries were fixed before creating this unique directory.
        assert_eq!(self.root.parent(), Some(self.parent.as_path()));
        assert!(
            self.root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("tree-move-")
        );
        fs::remove_dir_all(&self.root).unwrap();
    }
}
