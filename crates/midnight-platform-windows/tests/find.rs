#![cfg(windows)]
#[path = "../src/test_fixtures.rs"]
mod test_fixtures;
use midnight_core::*;
use midnight_platform_windows::WindowsFileSystem;
use std::{
    fs,
    io::{Read, Write},
    mem::size_of,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, FILE_BASIC_INFO, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES,
    FileBasicInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
};

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

/// Decided limit (VIEW-05, as for viewers): a same-size rewrite that restores the
/// LastWriteTime moves only the ChangeTime, which Find accepts so that endpoint
/// protection tagging a file does not fail it. The exact capture still tells them apart.
#[test]
fn native_find_accepts_same_size_edits_that_restore_the_last_write_time() {
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
    source.revalidate(&cancel).unwrap();
    let reopened = service.open_find_source(&path, &cancel).unwrap();
    assert!(!reopened.entry().matches_capture(source.entry()));
    assert!(
        reopened
            .entry()
            .matches_capture_except_change_time(source.entry())
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
fn native_find_leaves_dates_to_the_engine_locale_which_formats_them_like_listings() {
    let temporary = test_fixtures::build_temp("fixture-");
    let path = temporary.path().join("localized.txt");
    fs::write(&path, b"date metadata").unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let listed = service.entry_metadata(&path).unwrap();
    assert!(listed.localized_modified.is_some());
    // Neither an opened source nor an enumerated child is formatted per node...
    let source = service.open_find_source(&path, &cancel).unwrap();
    let mut directory = service.open_find_source(temporary.path(), &cancel).unwrap();
    let Some(FindChild::Entry(child)) = directory.next_child(&cancel).unwrap() else {
        panic!("the fixture file is enumerated");
    };
    for entry in [source.entry(), directory.entry(), child.entry()] {
        assert_eq!(entry.metadata.localized_modified, None, "{:?}", entry.path);
    }
    // ...and the engine formats a result's captured time with this native locale
    // exactly as the panel listing does.
    for entry in [source.entry(), child.entry()] {
        assert_eq!(entry.metadata.modified, listed.modified);
        assert_eq!(
            service.format_datetime(entry.metadata.modified.unwrap()),
            listed.localized_modified
        );
    }
}

/// Manual measurement: a name search of 200 folders of 200 empty files, enumerated and
/// opened like the engine's traversal; results (one name per folder) are revalidated
/// and get the engine's fill-when-absent date formatting. `every node` also formats each
/// unformatted node, the provider's former per-node work. Build before and after a
/// provider change and run `--ignored --nocapture localization_cost` on the same tree.
#[test]
#[ignore = "manual performance fixture; writes 40,000 empty native files under _build/qa"]
fn native_find_localization_cost_fixture() {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };
    const FOLDERS: usize = 200;
    const FILES: usize = 200;
    let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build/qa/operations-find/localization-cost");
    fs::create_dir_all(&parent).unwrap();
    let parent = fs::canonicalize(parent).unwrap();
    let root = parent.join("tree");
    if !parent.join("tree.complete").exists() {
        for folder in 0..FOLDERS {
            let folder = root.join(format!("folder-{folder}"));
            fs::create_dir_all(&folder).unwrap();
            for file in 0..FILES {
                fs::write(folder.join(format!("file-{file}.txt")), []).unwrap();
            }
        }
        fs::write(parent.join("tree.complete"), []).unwrap();
    }
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let result_name = std::ffi::OsStr::new("file-7.txt");
    let search = |every_node: bool| {
        let started = Instant::now();
        let (mut nodes, mut results, mut by_provider, mut formatted) = (0, 0, 0, 0);
        let mut stack = vec![service.open_find_source(&root, &cancel).unwrap()];
        while let Some(directory) = stack.last_mut() {
            let Some(child) = directory.next_child(&cancel).unwrap() else {
                stack.pop();
                continue;
            };
            let FindChild::Entry(source) = child else {
                panic!("fixture children are ordinary entries");
            };
            nodes += 1;
            let entry = source.entry();
            by_provider += usize::from(entry.metadata.localized_modified.is_some());
            let result = entry.path.file_name() == Some(result_name);
            if result {
                source.revalidate(&cancel).unwrap();
                results += 1;
            }
            if (result || every_node) && entry.metadata.localized_modified.is_none() {
                formatted += 1;
                black_box(service.format_datetime(entry.metadata.modified.unwrap()));
            }
            if entry.kind == EntryKind::Directory {
                stack.push(source);
            }
        }
        (started.elapsed(), nodes, results, by_provider, formatted)
    };
    search(false);
    let mut samples = [Vec::new(), Vec::new()];
    for round in 0..7 {
        for every_node in [round % 2 == 0, round % 2 != 0] {
            let (elapsed, nodes, results, by_provider, formatted) = search(every_node);
            eprintln!(
                "{}: {elapsed:?}, {nodes} nodes, {results} results, {by_provider} provider-formatted, {formatted} formatted here",
                if every_node {
                    "every node"
                } else {
                    "results only"
                }
            );
            samples[usize::from(every_node)].push(elapsed);
        }
    }
    for (label, mut times) in ["results only", "every node"].into_iter().zip(samples) {
        times.sort();
        eprintln!(
            "{label}: min {:?}, median {:?}",
            times[0],
            times[times.len() / 2]
        );
    }
    let base = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let started = Instant::now();
    for index in 0..(FOLDERS * FILES) as u64 {
        black_box(service.format_datetime(base + Duration::from_secs(index * 61)));
    }
    eprintln!(
        "{} date/time formats alone: {:?}",
        FOLDERS * FILES,
        started.elapsed()
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

/// Rewrite `path`'s basic information (times and attributes) through a handle; a zero
/// field is left as it is.
fn set_basic(path: &std::path::Path, change: impl FnOnce(&mut FILE_BASIC_INFO)) {
    let file = fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES | FILE_READ_ATTRIBUTES)
        .open(path)
        .unwrap();
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: a live handle and a writable FILE_BASIC_INFO of the queried size.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
    let mut wanted = FILE_BASIC_INFO {
        CreationTime: 0,
        LastAccessTime: 0,
        LastWriteTime: 0,
        ChangeTime: 0,
        FileAttributes: 0,
    };
    let mut current = basic;
    change(&mut current);
    if current.ChangeTime != basic.ChangeTime {
        wanted.ChangeTime = current.ChangeTime;
    }
    if current.LastWriteTime != basic.LastWriteTime {
        wanted.LastWriteTime = current.LastWriteTime;
    }
    if current.CreationTime != basic.CreationTime {
        wanted.CreationTime = current.CreationTime;
    }
    if current.FileAttributes != basic.FileAttributes {
        wanted.FileAttributes = current.FileAttributes;
    }
    // SAFETY: a live handle opened for attribute writes and a valid FILE_BASIC_INFO.
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileBasicInfo,
                (&wanted as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
}

/// Endpoint protection tags a freshly read file with extended attributes, which moves its
/// ChangeTime alone: Find still takes it for the file it captured, before and after
/// reading it. A new write time, creation time, attribute or size is still a change, and
/// names what changed.
#[test]
fn native_find_accepts_a_moved_change_time_alone() {
    let temporary = test_fixtures::build_temp("find-change-time-");
    let path = temporary.path().join("archive.zip");
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let reset = |path: &std::path::Path| {
        let _ = fs::remove_file(path);
        fs::write(path, b"content").unwrap();
    };

    reset(&path);
    let mut source = service.open_find_source(&path, &cancel).unwrap();
    set_basic(&path, |basic| basic.ChangeTime += 10_000_000);
    source.revalidate(&cancel).unwrap();
    let mut bytes = [0; 16];
    let count = source.read_chunk(&mut bytes, &cancel).unwrap();
    assert_eq!(&bytes[..count], b"content");
    set_basic(&path, |basic| basic.ChangeTime += 10_000_000);
    source.revalidate(&cancel).unwrap();
    let reopened = service.open_find_source(&path, &cancel).unwrap();
    assert!(
        reopened
            .entry()
            .matches_capture_except_change_time(source.entry())
    );
    assert!(!reopened.entry().matches_capture(source.entry()));

    type Change = fn(&std::path::Path);
    let changes: [(&str, Change); 4] = [
        ("last write time", |path| {
            set_basic(path, |basic| basic.LastWriteTime -= 10_000_000)
        }),
        ("creation time", |path| {
            set_basic(path, |basic| basic.CreationTime -= 10_000_000)
        }),
        ("attributes", |path| {
            set_basic(path, |basic| basic.FileAttributes |= FILE_ATTRIBUTE_HIDDEN)
        }),
        ("size", |path| {
            fs::OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(b"!")
                .unwrap()
        }),
    ];
    for (fact, change) in changes {
        reset(&path);
        let mut source = service.open_find_source(&path, &cancel).unwrap();
        change(&path);
        let error = source.revalidate(&cancel).unwrap_err();
        assert!(error.message.contains(fact), "{fact}: {}", error.message);
        let error = source.read_chunk(&mut bytes, &cancel).unwrap_err();
        assert!(error.message.contains(fact), "{fact}: {}", error.message);
        if fact == "attributes" {
            set_basic(&path, |basic| {
                basic.FileAttributes &= !FILE_ATTRIBUTE_HIDDEN
            });
        }
    }
}
