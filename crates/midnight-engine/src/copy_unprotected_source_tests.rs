//! Copies whose sources the platform cannot give exact lookups, into a folder that has
//! them: on Windows, from a volume that still creates 8.3 names (the system drive, so a
//! Downloads folder) to another NTFS volume that creates none. The sources stay unclaimed
//! (a Copy only reads them), and every change in the destination still goes ahead.
//!
//! A job claims the folders of its items. On a volume with exact lookups the fixtures name
//! that folder outside ASCII, which a case-insensitive parent cannot key exactly either:
//! its pin is unknown the same way. `cross_volume_copy_from_a_folder_without_exact_lookups`
//! takes a folder on a volume without exact lookups instead, when one is named.
use super::*;

/// A selection like a Downloads folder's: folders with nested content and files of
/// several kinds, one name with spaces and a comma.
fn downloads(folder: &Path) -> Vec<PathBuf> {
    let tool = folder.join("ovclash-0.2.2-windows-x86_64");
    for (path, size) in [
        ("ovclash.exe", 70_001),
        ("README.md", 900),
        ("lib/core.dll", 12_345),
        ("lib/plugins/extra.dll", 4_096),
        ("share/doc/LICENSE.txt", 1_100),
    ] {
        let path = tool.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![path.as_os_str().len() as u8; size]).unwrap();
    }
    let docs = folder.join("ovclash022-docs");
    fs::create_dir_all(docs.join("img")).unwrap();
    fs::write(docs.join("index.html"), b"<html></html>").unwrap();
    fs::write(docs.join("img/logo.png"), [7; 2_048]).unwrap();
    fs::create_dir_all(docs.join("empty")).unwrap();
    let mut selected = vec![tool, docs];
    for (file, size) in [
        ("lecture-notes.pdf", 31_000),
        ("fm-0.1.0.tar.gz", 65_537),
        ("fm-0.5.0.zip", 1),
        ("Report, final version 2026.pdf", 4_000),
        ("midnight-explorer-settings.json", 120),
        ("ovclash-0.2.2-py3-none-win_amd64.whl", 9_999),
        ("NOTES.md", 0),
        ("letter.docx", 14_271),
    ] {
        let path = folder.join(file);
        fs::write(&path, vec![size as u8; size]).unwrap();
        selected.push(path);
    }
    selected
}

/// Every file below `source` is in `copy` with the same bytes, and every folder too.
fn assert_same_tree(source: &Path, copy: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let copied = copy.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            assert!(copied.is_dir(), "{} was not copied", copied.display());
            assert_same_tree(&entry.path(), &copied);
        } else {
            assert_eq!(
                fs::read(&copied).unwrap(),
                fs::read(entry.path()).unwrap(),
                "{}",
                copied.display()
            );
        }
    }
}

/// Copies `selected` into an empty `destination` and checks that the job completed
/// without a question or an error, with every item and its contents in place.
async fn copy_completes(selected: &[PathBuf], destination: &Path) {
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, selected, destination))
        .await;
    let done = h.terminal(1).await;
    assert!(
        done.state == CopyOperationState::Completed && done.errors.is_empty(),
        "{:?}: {:#?}",
        done.state,
        done.errors
    );
    for item in selected {
        let copied = destination.join(item.file_name().unwrap());
        if item.is_dir() {
            assert_same_tree(item, &copied);
        } else {
            assert_eq!(fs::read(&copied).unwrap(), fs::read(item).unwrap());
        }
        assert!(item.exists(), "a Copy keeps its source");
    }
    h.close().await;
}

/// The owner's report: every item of a Downloads folder (folders and files) copied into
/// an empty folder named `!QQQ`; also one folder alone, files alone, and a destination
/// without the `!`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copies_of_items_without_exact_lookups_into_an_exact_folder_complete() {
    let temp = fixture();
    let source = temp.path().join("Sta\u{17e}en\u{e9}");
    let selected = downloads(&source);
    let cases: [(&str, Vec<PathBuf>); 4] = [
        ("!QQQ", selected.clone()),
        ("!one folder", selected[..1].to_vec()),
        ("!files", selected[2..].to_vec()),
        ("QQQ", selected.clone()),
    ];
    for (name, items) in cases {
        let destination = temp.path().join(name);
        fs::create_dir(&destination).unwrap();
        copy_completes(&items, &destination).await;
    }
}

/// More items than a job prepares before it starts: the job's footprint is then the
/// folder of the items (its name outside ASCII here) and the destination folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_copy_from_a_folder_without_exact_lookups_into_an_exact_folder_completes() {
    let temp = fixture();
    let source = temp.path().join("Sta\u{17e}en\u{e9}");
    let mut selected = downloads(&source);
    for index in selected.len()..=input::MAX_PREPARED_NATIVE_ROOTS {
        let file = source.join(format!("file {index:03}.txt"));
        fs::write(&file, format!("contents of {index}")).unwrap();
        selected.push(file);
    }
    let destination = temp.path().join("!QQQ");
    fs::create_dir(&destination).unwrap();
    copy_completes(&selected, &destination).await;
}

/// The owner's case itself, across volumes, with ASCII names: set
/// `MIDNIGHT_TEST_UNQUALIFIED_ROOT` to a folder on a volume without exact lookups (on
/// Windows, one that still creates 8.3 names, such as the system drive), while TEMP is on
/// a volume with them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs MIDNIGHT_TEST_UNQUALIFIED_ROOT on another volume"]
async fn cross_volume_copy_from_a_folder_without_exact_lookups() {
    let root = PathBuf::from(
        std::env::var_os("MIDNIGHT_TEST_UNQUALIFIED_ROOT")
            .expect("MIDNIGHT_TEST_UNQUALIFIED_ROOT names a folder"),
    );
    let source = tempfile::Builder::new()
        .prefix("Downloads-")
        .tempdir_in(root)
        .unwrap();
    let selected = downloads(source.path());
    let temp = fixture();
    let destination = temp.path().join("!QQQ");
    fs::create_dir(&destination).unwrap();
    copy_completes(&selected, &destination).await;
}
