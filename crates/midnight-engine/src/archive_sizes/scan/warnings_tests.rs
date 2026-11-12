use super::tests::{Mount, node, run};
use super::*;
use std::{collections::HashMap, sync::Arc};

fn named(id: u64, parent: u64, kind: EntryKind, name: &[u8]) -> ArchiveNode {
    let mut value = node(id, parent, kind, None);
    value.raw_name = Arc::from(name);
    value.display_name = "lossy display alias".into();
    value
}

fn text(size: &FolderSize) -> String {
    let report = size
        .warnings
        .as_ref()
        .expect("actual issue retains a report");
    let source = report.snapshot(&Cancellation::default()).unwrap();
    let mut output = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = source.read_at(output.len() as u64, &mut buffer).unwrap();
        if count == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..count]);
    }
    let text = String::from_utf8(output).unwrap();
    assert_eq!(
        report.summary().stored,
        report.summary().observed,
        "scan fixture must retain all diagnostic records: {text}"
    );
    text
}

#[test]
fn warning_ranges_include_each_descendant_once_and_exclude_later_siblings() {
    let mut mount = Mount::default();
    mount.rows.insert(
        2,
        vec![
            named(3, 2, EntryKind::Directory, b"denied-A"),
            named(4, 2, EntryKind::Directory, b"branch-B"),
            named(5, 2, EntryKind::File, b"root-unknown"),
        ],
    );
    mount
        .rows
        .insert(4, vec![named(6, 4, EntryKind::File, b"child-unknown")]);
    let (ok, sizes) = run(
        &mount,
        &[named(2, 1, EntryKind::Directory, "root 文档".as_bytes())],
    );
    assert!(ok);
    assert_eq!(sizes[&2].errors, 3);
    let parent = text(&sizes[&2]);
    assert!(parent.contains("root 文档"));
    assert!(!parent.contains("lossy display alias"));
    assert_eq!(parent.matches("Reason:").count(), 3);
    assert!(parent.contains("unreadable"));
    assert!(parent.contains("root-unknown"));
    assert!(parent.contains("child-unknown"));
    let first = text(&sizes[&3]);
    assert!(first.contains("denied-A"));
    assert!(!first.contains("branch-B"));
    assert!(!first.contains("root-unknown"));
    assert_eq!(first.matches("Reason:").count(), 1);
    let second = text(&sizes[&4]);
    assert!(second.contains("branch-B"));
    assert!(second.contains("child-unknown"));
    assert!(!second.contains("denied-A"));
    assert_eq!(second.matches("Reason:").count(), 1);
}

#[test]
fn cancelling_after_a_real_issue_retains_its_partial_report_without_sibling_bleed() {
    let cancel = Cancellation::default();
    let mut mount = Mount {
        cancel: Some(cancel.clone()),
        cancel_on: Some(3),
        ..Mount::default()
    };
    mount.rows.insert(
        2,
        vec![
            named(4, 2, EntryKind::File, b"missing-length"),
            named(3, 2, EntryKind::Directory, b"cancelled-child"),
        ],
    );
    mount.rows.insert(3, vec![]);
    let mut sizes = HashMap::new();
    let mut early = None;
    assert!(!compute(
        &mount,
        &tests::address(),
        &[1],
        &[],
        &[named(2, 1, EntryKind::Directory, b"root")],
        &cancel,
        &mut |id, size| {
            if early.is_none() && size.errors != 0 {
                early = Some(size.clone());
            }
            sizes.insert(id.entry, size);
            true
        }
    ));
    let root = &sizes[&2];
    assert!(!root.finished);
    assert_eq!(root.errors, 1);
    assert_eq!(root.files, 1);
    assert!(text(root).contains("Scan cancelled"));
    assert!(text(root).contains("missing-length"));
    assert_eq!(text(root).matches("Reason:").count(), 1);
    assert!(
        text(&early.unwrap()).contains("Scan running"),
        "earlier handles stay immutable"
    );
    assert!(sizes[&3].warnings.is_none());
}

#[test]
fn raw_names_control_characters_and_invalid_components_are_unambiguous() {
    let mut mount = Mount::default();
    mount.rows.insert(
        2,
        vec![
            named(3, 2, EntryKind::File, b"raw-\xff\nname"),
            named(4, 2, EntryKind::Directory, b"../escape"),
            node(5, 2, EntryKind::File, Some(19)),
        ],
    );
    let (ok, sizes) = run(&mount, &[named(2, 1, EntryKind::Directory, b"root")]);
    assert!(ok);
    assert_eq!(
        sizes[&2].bytes, 19,
        "invalid paths do not stop useful siblings"
    );
    assert_eq!(sizes[&2].errors, 2);
    let report = text(&sizes[&2]);
    assert!(report.contains("\\xff"));
    assert!(report.contains("\\n"));
    assert!(report.contains("../escape"));
    assert!(report.contains("Invalid archive member"));
    assert_eq!(mount.lists.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn final_archive_revision_diagnostic_retains_error_and_never_finishes_a_clean_root() {
    use std::sync::atomic::Ordering;
    let mut mount = Mount::default();
    mount
        .rows
        .insert(2, vec![node(3, 2, EntryKind::File, Some(19))]);
    let mut last = FolderSize::default();
    let result = compute_checked(
        &mount,
        ScanScope {
            address: &tests::address(),
            ancestors: &[1],
            reserved: &[],
        },
        &[named(2, 1, EntryKind::Directory, b"root")],
        &Cancellation::default(),
        &mut |id, size| {
            if id.entry == 2 && mount.lists.load(Ordering::Relaxed) != 0 {
                mount.changed.store(true, Ordering::Relaxed);
            }
            last = size;
            true
        },
        &crate::ReportArena::testing(),
    );
    let error = result.expect_err("the final revision error must survive without revalidation");
    mount.changed.store(false, Ordering::Relaxed);
    assert_eq!(error.message, "changed");
    assert!(!last.finished);
    assert_eq!(last.bytes, 19);
    assert_eq!(last.errors, 1);
    let report = text(&last);
    assert!(report.contains("root"));
    assert!(report.contains("changed"));
    assert_eq!(report.matches("Reason:").count(), 1);
}

#[derive(Debug)]
struct CancellingFailure {
    cancel: Cancellation,
    kind: io::ErrorKind,
    validation: usize,
    validations: std::sync::atomic::AtomicUsize,
}
impl CancellingFailure {
    fn failure(&self) -> FsError {
        self.cancel.cancel();
        FsError::from_io(
            "archive diagnostic race",
            Path::new("captured-provider-path"),
            io::Error::new(self.kind, "exact concurrent provider reason"),
        )
    }
}
impl ArchiveMount for CancellingFailure {
    fn id(&self) -> u64 {
        7
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        if self.validation == 0 {
            Err(self.failure())
        } else {
            Ok(vec![node(3, 2, EntryKind::File, Some(19))])
        }
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("metadata scan cannot read archive content")
    }
    fn validate(&self) -> Result<(), FsError> {
        let count = self
            .validations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if count == self.validation {
            Err(self.failure())
        } else {
            Ok(())
        }
    }
}

fn concurrent_failure(kind: io::ErrorKind, validation: usize) -> FolderSize {
    let mount = CancellingFailure {
        cancel: Cancellation::default(),
        kind,
        validation,
        validations: Default::default(),
    };
    let mut last = FolderSize::default();
    let result = compute_checked(
        &mount,
        ScanScope {
            address: &tests::address(),
            ancestors: &[1],
            reserved: &[],
        },
        &[named(2, 1, EntryKind::Directory, b"raw-root")],
        &mount.cancel,
        &mut |_, size| {
            last = size;
            true
        },
        &crate::ReportArena::testing(),
    );
    assert!(!matches!(result, Ok(true)));
    assert!(!last.finished);
    last
}

#[test]
fn archive_diagnostic_keeps_real_provider_failures_that_race_cancellation() {
    // The list call, first validation and final validation each own the error
    // they return, even if cancellation becomes visible during that call.
    for validation in [0, 1, 4] {
        let size = concurrent_failure(io::ErrorKind::PermissionDenied, validation);
        assert_eq!(size.errors, 1, "provider boundary {validation}");
        let report = text(&size);
        assert!(report.contains("PermissionDenied"));
        assert!(report.contains("exact concurrent provider reason"));
        assert!(report.contains("raw-root"));
        assert!(report.contains("Scan cancelled"));
        assert_eq!(report.matches("Reason:").count(), 1);
    }
}

#[test]
fn archive_diagnostic_does_not_turn_interrupted_cancellation_into_an_issue() {
    for validation in [0, 1, 4] {
        let size = concurrent_failure(io::ErrorKind::Interrupted, validation);
        assert_eq!(size.errors, 0, "provider boundary {validation}");
        assert!(size.warnings.is_none());
        if validation == 4 {
            assert_eq!(size.bytes, 19, "already counted bytes survive cancellation");
        }
    }
}
