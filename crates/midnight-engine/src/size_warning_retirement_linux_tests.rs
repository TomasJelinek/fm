use super::*;
use midnight_core::TreeDiagnostic;
use std::os::unix::fs::MetadataExt;

/// The descriptor of the report file: its `/proc/self/fd` entry, the test's own private
/// folder that the entry's link names, and the file's device, inode and length. Once the
/// file is closed, another thread's next file can take both the same descriptor number
/// and the freed inode number. The folder tells the report from a newcomer of another
/// test, which uses a unique temporary folder of its own, and the length (the report is
/// hundreds of kilobytes) from any other file.
struct Descriptor {
    entry: PathBuf,
    folder: PathBuf,
    device: u64,
    inode: u64,
    length: u64,
}
fn spool_descriptor(directory: &Path) -> Descriptor {
    let found: Vec<_> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let target = std::fs::read_link(&path).ok()?;
            let metadata = std::fs::metadata(&path).ok()?;
            (target.starts_with(directory) && metadata.is_file()).then(|| Descriptor {
                entry: path,
                folder: directory.to_owned(),
                device: metadata.dev(),
                inode: metadata.ino(),
                length: metadata.len(),
            })
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one actual anonymous report file is shared by its prefixes"
    );
    found.into_iter().next().unwrap()
}
/// Whether the descriptor still holds the report file: a file in the test's own folder
/// with the report's device, inode and length.
fn same_descriptor(descriptor: &Descriptor) -> bool {
    std::fs::read_link(&descriptor.entry).is_ok_and(|target| target.starts_with(&descriptor.folder))
        && std::fs::metadata(&descriptor.entry).is_ok_and(|metadata| {
            (metadata.dev(), metadata.ino(), metadata.len())
                == (descriptor.device, descriptor.inode, descriptor.length)
        })
}
#[test]
fn report_retirement_tree_diagnostic_prefix_keeps_file_until_worker_cleanup() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("private");
    let (make_log, observer) = observed(directory.clone());
    let (log, diagnostic) = thread::spawn(move || {
        let log = make_log();
        let reason = format!("{} final diagnostic tail", "東京 failure ".repeat(20_000));
        let source = crate::tree_diagnostics::report(&log, "資料 folder", &reason);
        assert!(contents(source.as_ref()).contains(&reason));
        let diagnostic = TreeDiagnostic::with_details("bounded summary", source);
        (log, diagnostic)
    })
    .join()
    .unwrap();
    let descriptor = spool_descriptor(&directory);
    drop(log);
    assert!(
        same_descriptor(&descriptor),
        "the text prefix must retain the actual file"
    );
    let retained = diagnostic.clone();
    drop(diagnostic);
    assert!(
        same_descriptor(&descriptor),
        "another diagnostic still owns the prefix"
    );
    drop(retained);
    // The native destructor is gated before AppendSpool's actual file field drops.
    // The baseline instead closes this final SpoolPrefix synchronously here.
    assert!(
        same_descriptor(&descriptor),
        "last diagnostic closed its native file on caller"
    );
    let actual = observer
        .dropped
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_ne!(
        actual,
        thread::current().id(),
        "native storage cleanup must run off caller"
    );
    observer.finish();
    assert!(
        !same_descriptor(&descriptor),
        "actual cleanup must close the final descriptor"
    );
}
