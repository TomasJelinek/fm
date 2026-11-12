use super::tests::{Tree, item};
use super::*;

fn report_text(size: &FolderSize) -> String {
    let report = size
        .warnings
        .as_ref()
        .expect("every counted issue has details");
    assert_eq!(report.summary().observed, size.errors);
    let source = report.snapshot(&Cancellation::default()).unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = source.read_at(bytes.len() as u64, &mut buffer).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn native_directory_and_metadata_failures_keep_exact_paths_and_reasons() {
    let mut tree = Tree::default();
    tree.directory(
        "root",
        vec![
            item(2, "root/denied 資料", EntryKind::Directory),
            item(3, "root/missing metadata", EntryKind::File),
            item(4, "root/good", EntryKind::File),
        ],
    );
    tree.unreadable.insert("root/denied 資料".into());
    tree.length("root/good", 17);
    let mut last = FolderSize::default();
    compute(
        &tree,
        &[item(1, "root", EntryKind::Directory)],
        &Cancellation::default(),
        &mut |_, size| {
            last = size;
            true
        },
    );
    assert!(last.finished);
    assert_eq!((last.bytes, last.errors), (17, 2));
    let text = report_text(&last);
    assert!(text.contains("root/denied 資料"));
    assert!(text.contains("resolve directory size target"));
    assert!(text.contains("PermissionDenied"));
    assert!(text.contains("root/missing metadata"));
    assert!(text.contains("NotFound"));
}

#[test]
fn cancellation_preserves_earlier_issues_without_inventing_a_cancellation_error() {
    let cancel = Cancellation::default();
    let mut tree = Tree::default();
    tree.cancel_on_metadata = Some(cancel.clone());
    // This invalid child is recorded before the metadata call cancels the scan.
    tree.directory(
        "root",
        vec![
            item(2, "outside", EntryKind::File),
            item(3, "root/a", EntryKind::File),
        ],
    );
    tree.length("root/a", 10);
    let mut last = FolderSize::default();
    compute(
        &tree,
        &[item(1, "root", EntryKind::Directory)],
        &cancel,
        &mut |_, size| {
            last = size;
            true
        },
    );
    assert!(!last.finished);
    assert_eq!(last.errors, 1);
    let text = report_text(&last);
    assert!(text.contains("outside"));
    assert!(text.contains("Scan cancelled"));
    assert!(!text.contains("Kind: Interrupted"));
}

#[test]
fn unavailable_report_storage_never_stops_counting_or_hides_missing_details() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("occupied");
    std::fs::write(&path, b"occupied").unwrap();
    let log = ReportLog::new(&crate::ReportArena::testing(), path);
    let mut tree = Tree::default();
    let mut entries = (0..300)
        .map(|id| item(id + 2, &format!("outside-{id}"), EntryKind::File))
        .collect::<Vec<_>>();
    entries.push(item(500, "root/good", EntryKind::File));
    tree.directory("root", entries);
    tree.length("root/good", 42);
    let mut last = FolderSize::default();
    assert!(scan_root(
        &tree,
        &item(1, "root", EntryKind::Directory),
        &Cancellation::default(),
        &mut |_, size| {
            last = size;
            true
        },
        PENDING_PATH_BUDGET,
        &log
    ));
    assert_eq!((last.bytes, last.errors), (42, 300));
    let report = last.warnings.as_ref().unwrap();
    assert_eq!(
        (report.summary().observed, report.summary().stored),
        (300, 0)
    );
    assert!(report.summary().tooltip.contains("Report storage failed"));
    let text = report_text(&last);
    assert!(text.contains("300 issue details could not be stored"));
}

#[test]
fn batched_cancel_drains_the_final_partial_report() {
    let cancel = Cancellation::default();
    let mut tree = Tree::default();
    tree.cancel_on_metadata = Some(cancel.clone());
    tree.directory(
        "root",
        vec![
            item(2, "outside", EntryKind::File),
            item(3, "root/a", EntryKind::File),
        ],
    );
    tree.length("root/a", 10);
    let mut last = FolderSize::default();
    compute_batched(
        &tree,
        &[item(1, "root", EntryKind::Directory)],
        &cancel,
        &mut |_, size| {
            last = size;
            true
        },
        &std::sync::Arc::new(IoBudget::new()),
        &crate::ReportArena::testing(),
    );
    assert!(!last.finished);
    assert_eq!(last.errors, 1);
    assert!(report_text(&last).contains("outside"));
}
