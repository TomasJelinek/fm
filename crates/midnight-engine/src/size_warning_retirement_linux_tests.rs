use super::*;
use midnight_core::TreeDiagnostic;
use std::os::unix::fs::MetadataExt;

fn spool_descriptor(directory: &Path) -> (PathBuf, u64, u64) {
    let found: Vec<_> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let target = std::fs::read_link(&path).ok()?;
            let metadata = std::fs::metadata(&path).ok()?;
            (target.starts_with(directory) && metadata.is_file()).then_some((
                path,
                metadata.dev(),
                metadata.ino(),
            ))
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one actual anonymous report file is shared by its prefixes"
    );
    found.into_iter().next().unwrap()
}
fn same_descriptor(descriptor: &(PathBuf, u64, u64)) -> bool {
    std::fs::metadata(&descriptor.0)
        .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == (descriptor.1, descriptor.2))
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
