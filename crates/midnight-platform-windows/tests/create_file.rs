#![cfg(windows)]

use midnight_core::{Cancellation, MutationService};
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::OsStr,
    fs, io,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::Path,
    process::Command,
    sync::{Arc, Barrier},
};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("midnight-create-file-")
        .tempdir_in(root)
        .unwrap()
}

#[test]
fn creates_an_empty_unicode_file_with_its_exact_native_basename() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let name = OsStr::new("文档 🦀 e\u{301}.txt");
    let path = base.create_file(name, &Cancellation::default()).unwrap();
    assert_eq!(path, temp.path().join(name));
    assert!(fs::symlink_metadata(&path).unwrap().is_file());
    assert_eq!(fs::read(&path).unwrap(), b"");
    assert_eq!(
        fs::read_dir(temp.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name(),
        name
    );
}

#[test]
fn file_and_directory_collisions_preserve_existing_contents() {
    let temp = fixture();
    fs::write(temp.path().join("existing.txt"), b"keep original bytes").unwrap();
    fs::create_dir(temp.path().join("existing directory")).unwrap();
    fs::write(
        temp.path().join("existing directory/sentinel"),
        b"keep directory contents",
    )
    .unwrap();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    for name in ["existing.txt", "existing directory"] {
        assert!(
            base.create_file(OsStr::new(name), &Cancellation::default())
                .is_err()
        );
    }
    assert_eq!(
        fs::read(temp.path().join("existing.txt")).unwrap(),
        b"keep original bytes"
    );
    assert_eq!(
        fs::read(temp.path().join("existing directory/sentinel")).unwrap(),
        b"keep directory contents"
    );
}

#[test]
fn case_variant_creation_matches_ordinary_windows_lookup_without_overwriting() {
    let temp = fixture();
    let original = temp.path().join("case-name.txt");
    let alias = temp.path().join("CASE-NAME.TXT");
    let contents = b"original nonempty bytes";
    fs::write(&original, contents).unwrap();
    // Observe this directory's behavior through the ordinary Windows file-open path.
    // No global or per-directory case-sensitivity setting is changed by the test.
    let alias_resolves = match fs::read(&alias) {
        Ok(bytes) => {
            assert_eq!(bytes, contents);
            true
        }
        Err(error) => {
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
            false
        }
    };
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let result = base.create_file(OsStr::new("CASE-NAME.TXT"), &Cancellation::default());
    if alias_resolves {
        assert_eq!(result.unwrap_err().kind, io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&alias).unwrap(), contents);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    } else {
        assert_eq!(result.unwrap(), alias);
        assert_eq!(fs::read(&alias).unwrap(), b"");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }
    assert_eq!(fs::read(original).unwrap(), contents);
}

#[test]
fn cancellation_creates_no_file() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        base.create_file(OsStr::new("cancelled.txt"), &cancel)
            .unwrap_err()
            .kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn only_one_valid_basename_is_accepted_before_any_creation() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "one/two",
        "one\\two",
        "C:\\escape",
        "C:escape",
        "\\root",
        "\\\\server\\share\\file",
        "file:stream",
        "NUL.txt",
        "COM1",
        "LPT².txt",
        "trailing.",
        "trailing ",
        "bad*name",
        "bad\0name",
    ] {
        assert_eq!(
            base.create_file(OsStr::new(name), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidInput,
            "{name:?}"
        );
    }
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn long_parent_paths_have_no_application_max_path_limit() {
    let temp = fixture();
    let mut parent = temp.path().to_owned();
    for _ in 0..10 {
        parent.push("文档-long-parent-0123456789");
    }
    fs::create_dir_all(&parent).unwrap();
    assert!(parent.as_os_str().encode_wide().count() > 260);
    let base = WindowsFileSystem::new(None)
        .prepare_create(&parent, &Cancellation::default())
        .unwrap();
    let created = base
        .create_file(OsStr::new("empty 🦀.txt"), &Cancellation::default())
        .unwrap();
    assert_eq!(fs::metadata(created).unwrap().len(), 0);
}

#[test]
fn concurrent_same_name_creation_has_exactly_one_winner() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let base = base.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    base.create_file(OsStr::new("only-once.txt"), &Cancellation::default())
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| error.kind == io::ErrorKind::AlreadyExists)
    );
    assert_eq!(fs::read(temp.path().join("only-once.txt")).unwrap(), b"");
}

#[test]
fn replacing_the_prepared_base_does_not_redirect_file_creation() {
    let temp = fixture();
    let parent = temp.path().join("base");
    let original = temp.path().join("original");
    fs::create_dir(&parent).unwrap();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&parent, &Cancellation::default())
        .unwrap();
    fs::rename(&parent, &original).unwrap();
    fs::create_dir(&parent).unwrap();
    assert!(
        base.create_file(OsStr::new("must-not-exist"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&original).unwrap().count(), 0);
}

#[test]
fn existing_junction_is_not_followed_or_replaced() {
    let temp = fixture();
    let target = temp.path().join("outside target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("sentinel"), b"keep target intact").unwrap();
    let link = temp.path().join("junction");
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../midnight-platform/tests/create-junction.ps1"),
        )
        .arg("-JunctionPath")
        .arg(&link)
        .arg("-TargetPath")
        .arg(&target)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    assert!(
        base.create_file(OsStr::new("junction"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(
        fs::read(link.join("sentinel")).unwrap(),
        b"keep target intact"
    );
    assert_eq!(
        fs::read(target.join("sentinel")).unwrap(),
        b"keep target intact"
    );
}
