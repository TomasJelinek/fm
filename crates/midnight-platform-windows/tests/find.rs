#![cfg(windows)]
#[path = "../src/test_fixtures.rs"]
mod test_fixtures;
use midnight_core::*;
use midnight_platform_windows::WindowsFileSystem;
use std::{fs, io::Read};

#[test]
fn native_find_streams_roots_and_children_and_rejects_changed_results() {
    let temporary = test_fixtures::build_temp("fixture-");
    let root = temporary.path();
    let path = root.join("Find 文档 🦀.txt");
    fs::write(&path, "\u{feff}first\nčeský obsah").unwrap();
    fs::create_dir(root.join("subdirectory")).unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut directory = service.open_find_source(root, &cancel).unwrap();
    let mut children = Vec::new();
    while let Some(child) = directory.next_child(&cancel).unwrap() {
        if let FindChild::Entry(source) = child {
            children.push(source.entry().path.clone());
        }
    }
    assert!(children.contains(&path));
    assert!(children.contains(&root.join("subdirectory")));
    let mut source = service.open_find_source(&path, &cancel).unwrap();
    let captured = source.entry().clone();
    let mut bytes = [0; 128];
    let count = source.read_chunk(&mut bytes, &cancel).unwrap();
    assert_eq!(&bytes[..count], "\u{feff}first\nčeský obsah".as_bytes());
    source.revalidate(&cancel).unwrap();
    drop(source);
    fs::remove_file(&path).unwrap();
    fs::write(&path, "replacement object").unwrap();
    let replacement = service.open_find_source(&path, &cancel).unwrap();
    assert!(!replacement.entry().matches_capture(&captured));
    let drive = root.ancestors().last().unwrap();
    assert_eq!(
        service
            .open_find_source(drive, &cancel)
            .unwrap()
            .entry()
            .kind,
        EntryKind::Directory
    );
    let mut expected = Vec::new();
    fs::File::open(&path)
        .unwrap()
        .read_to_end(&mut expected)
        .unwrap();
    assert_eq!(expected, b"replacement object");
}

#[test]
fn native_find_keeps_enumerating_a_directory_that_changes_during_the_search() {
    let temporary = test_fixtures::build_temp("find-changing-");
    let root = temporary.path();
    let names: Vec<_> = (0..8).map(|index| format!("entry-{index}.txt")).collect();
    for name in &names {
        fs::write(root.join(name), b"x").unwrap();
    }
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested").join("inner.txt"), b"x").unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut directory = service.open_find_source(root, &cancel).unwrap();
    let captured = directory.entry().clone();
    let entry_path = |child: FindChild| match child {
        FindChild::Entry(source) => source.entry().path.clone(),
        _ => panic!("ordinary fixture entries are found"),
    };
    let mut seen = vec![entry_path(directory.next_child(&cancel).unwrap().unwrap())];
    // Another process adds an entry: the directory's write time, change time and size
    // change, which must not abandon the rest of the directory or its subtrees.
    fs::write(root.join("added-during-search.txt"), b"new").unwrap();
    while let Some(child) = directory.next_child(&cancel).unwrap() {
        seen.push(entry_path(child));
    }
    for name in &names {
        assert!(seen.contains(&root.join(name)), "{name} was skipped");
    }
    assert!(seen.contains(&root.join("nested")));
    // The directory is still the same Find result.
    directory.revalidate(&cancel).unwrap();
    let reopened = service.open_find_source(root, &cancel).unwrap();
    assert!(reopened.entry().matches_capture(&captured));
}

#[test]
fn native_find_rejects_cancelled_or_special_namespace_roots() {
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    assert!(
        service
            .open_find_source(std::path::Path::new(r"\\.\NUL"), &cancel)
            .is_err()
    );
    cancel.cancel();
    assert!(
        service
            .open_find_source(std::path::Path::new(r"C:\"), &cancel)
            .is_err()
    );
}

#[test]
fn native_find_rejects_same_size_edits_even_when_last_write_time_is_restored() {
    let temporary = test_fixtures::build_temp("fixture-");
    let path = temporary.path().join("restored-time.txt");
    fs::write(&path, b"AAAA").unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service.open_find_source(&path, &cancel).unwrap();
    let modified = source.entry().metadata.modified.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    fs::write(&path, b"BBBB").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(
        source.revalidate(&cancel).is_err(),
        "Same-size content with restored LastWriteTime must not validate as the captured result"
    );
}

#[test]
fn native_find_content_can_read_an_idle_log_without_denying_its_writer() {
    use std::{
        io::{Seek, SeekFrom, Write},
        os::windows::fs::OpenOptionsExt,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let temporary = test_fixtures::build_temp("fixture-");
    let path = temporary.path().join("open-log.txt");
    fs::write(&path, b"original needle\n").unwrap();
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = service.open_find_source(&path, &cancel).unwrap();
    let mut bytes = [0; 128];
    let count = source
        .read_chunk(&mut bytes, &cancel)
        .expect("Find must permit an existing idle log writer");
    assert_eq!(&bytes[..count], b"original needle\n");
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"new log entry\n").unwrap();
    writer.flush().unwrap();
    assert!(
        source.revalidate(&cancel).is_err(),
        "A concurrent write must invalidate the captured content revision"
    );
}

#[test]
fn native_find_metadata_uses_existing_backend_locale_date_and_time() {
    let temporary = test_fixtures::build_temp("fixture-");
    let path = temporary.path().join("localized.txt");
    fs::write(&path, b"date metadata").unwrap();
    let service = WindowsFileSystem::new(None);
    let metadata = service.entry_metadata(&path).unwrap();
    assert!(metadata.localized_modified.is_some());
    let source = service
        .open_find_source(&path, &Cancellation::default())
        .unwrap();
    assert_eq!(
        source.entry().metadata.localized_modified,
        metadata.localized_modified
    );
}

#[test]
fn native_find_nested_child_capture_matches_untouched_absolute_reopen() {
    use std::time::{Duration, SystemTime};

    let fixture_parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build/tmp/find-result-revision/windows");
    fs::create_dir_all(&fixture_parent).unwrap();
    let fixture = tempfile::tempdir_in(fs::canonicalize(fixture_parent).unwrap()).unwrap();
    let root = fixture.path().join("source");
    let parent = root.join("nested 文档");
    fs::create_dir_all(&parent).unwrap();
    let target = parent.join("DynamicMatrixCacheLimit.h");
    fs::write(&target, vec![b'x'; 2670]).unwrap();
    let modified = SystemTime::UNIX_EPOCH + Duration::new(1_729_610_254, 308_620_000);
    fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut ancestors = vec![service.open_find_source(&root, &cancel).unwrap()];
    for path in [&parent, &target] {
        let directory = ancestors.last_mut().unwrap();
        let child = loop {
            match directory.next_child(&cancel).unwrap() {
                Some(FindChild::Entry(child)) if child.entry().path == *path => break child,
                Some(FindChild::Failed(error)) => panic!("native child capture failed: {error:?}"),
                Some(_) => {}
                None => panic!("owned fixture child was not found: {path:?}"),
            }
        };
        ancestors.push(child);
    }
    let captured = ancestors.last().unwrap().entry().clone();
    assert_eq!(captured.path, target);
    assert_eq!(captured.kind, EntryKind::File);
    assert_eq!(captured.metadata.size, Some(2670));
    assert_eq!(captured.metadata.modified, Some(modified));
    let assert_reopen = || {
        let current = service.open_find_source(&target, &cancel).unwrap();
        assert_eq!(current.entry().revision, captured.revision);
        assert!(current.entry().matches_capture(&captured));
        current.revalidate(&cancel).unwrap();
    };
    for _ in 0..3 {
        assert_reopen();
        ancestors.last().unwrap().revalidate(&cancel).unwrap();
    }
    drop(ancestors);
    for _ in 0..3 {
        assert_reopen();
    }
}
