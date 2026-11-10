use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

// This unit test reaches the actual native boundary between preflight stat and
// content open, so replacement is deterministic rather than a timing-dependent race.
#[test]
fn special_or_regular_replacement_after_stat_cannot_become_the_captured_file() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let base = repository.join("_build/goal/native-macos/fixtures");
    fs::create_dir_all(&base).unwrap();
    let directory = base.join(format!(
        "race-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let source = directory.join("source");
    fs::write(&source, b"original").unwrap();
    let parent = Parent {
        directory: File::open(&directory).unwrap(),
        name: c"source".into(),
        requires_directory: false,
    };
    let before = parent.stat().unwrap();
    fs::rename(&source, directory.join("kept-original")).unwrap();
    let name = CString::new(source.as_os_str().as_bytes()).unwrap();
    // SAFETY: freshly vacated owned fixture name, owner-only permissions.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let error = open_checked(&parent, &before).unwrap_err();
    assert_eq!(error.code, ErrorCode::SourceChanged);
    fs::remove_file(&source).unwrap();
    fs::write(&source, b"replaced").unwrap();
    let error = open_checked(&parent, &before).unwrap_err();
    assert_eq!(error.code, ErrorCode::SourceChanged);
    assert_eq!(
        fs::read(directory.join("kept-original")).unwrap(),
        b"original"
    );
    fs::remove_file(source).unwrap();
    fs::remove_file(directory.join("kept-original")).unwrap();
    drop(parent);
    fs::remove_dir(directory).unwrap();
}
