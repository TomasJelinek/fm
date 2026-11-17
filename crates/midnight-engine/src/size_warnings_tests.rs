use super::*;

fn log() -> (tempfile::TempDir, ReportLog) {
    let temporary = tempfile::tempdir().unwrap();
    let log = ReportLog::new(&ReportArena::testing(), temporary.path().join("warnings"));
    (temporary, log)
}

fn record(log: &ReportLog, name: &str) {
    log.record(
        format_args!("{:?}", Path::new(name)),
        "read directory",
        io::ErrorKind::PermissionDenied,
        "Access denied",
    );
}

fn text(source: &dyn GeneratedTextSource) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 997];
        let count = source.read_at(bytes.len() as u64, &mut chunk).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    assert_eq!(bytes.len() as u64, source.length());
    String::from_utf8(bytes).unwrap()
}

#[test]
fn handles_freeze_publication_counts_and_first_actual_reason() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    assert!(report.handle().is_none());
    record(&log, "first 資料");
    let first = report.handle().unwrap();
    assert_eq!(first.summary().observed, 1);
    assert_eq!(first.summary().stored, 1);
    assert!(first.summary().tooltip.contains("Access denied"));
    assert!(first.summary().tooltip.contains("first 資料"));
    record(&log, "second");
    report.finish(false);
    let final_report = report.handle().unwrap();
    assert_eq!(final_report.summary().observed, 2);
    let old = text(first.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(!old.contains("second"));
    assert!(old.contains("Scan running"));
}

#[test]
fn first_reason_is_per_range_and_finished_empty_ranges_release_summary_slots() {
    let (_temporary, log) = log();
    for _ in 0..100 {
        log.mark().report().finish(false);
    }
    assert!(log.0.lock().unwrap().first_issues.is_empty());
    let first = log.mark().report();
    record(&log, "first");
    first.finish(false);
    let second = log.mark().report();
    record(&log, "second");
    second.finish(false);
    assert!(
        second
            .handle()
            .unwrap()
            .summary()
            .tooltip
            .contains("second")
    );
    assert!(!second.handle().unwrap().summary().tooltip.contains("first"));
    assert!(first.handle().unwrap().summary().tooltip.contains("first"));
}

#[test]
fn custom_diagnostic_title_does_not_describe_a_size_scan() {
    let temporary = tempfile::tempdir().unwrap();
    let log = ReportLog::with_title(
        &ReportArena::testing(),
        temporary.path().join("details"),
        "Folder listing error",
    );
    let report = log.mark().report();
    record(&log, "unreadable");
    report.finish(false);
    let contents = text(report.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(contents.starts_with("Folder listing error\nReport finished"));
}

#[test]
fn report_snapshots_keep_unicode_paths_reasons_and_exact_old_boundaries() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    record(&log, "first 資料 🦀");
    let first = report.snapshot(&Cancellation::default()).unwrap();
    record(&log, "second");
    report.finish(false);
    let final_source = report.snapshot(&Cancellation::default()).unwrap();
    drop(report);
    drop(log);
    let old = text(first.as_ref());
    assert!(old.contains("first 資料 🦀"));
    assert!(old.contains("PermissionDenied"));
    assert!(old.contains("Access denied"));
    assert!(!old.contains("second"));
    assert!(old.contains("Observed issues: 1"));
    let final_text = text(final_source.as_ref());
    assert!(final_text.contains("second"));
    assert!(final_text.contains("Observed issues: 2"));
    assert!(final_text.contains("Stored details: 2"));
    assert!(final_text.contains("Scan finished"));
}

#[test]
fn subtree_ranges_include_descendants_once_and_exclude_later_siblings() {
    let (_temporary, log) = log();
    let root = log.mark().report();
    record(&log, "root issue");
    let child = log.mark().report();
    record(&log, "child issue");
    child.finish(false);
    record(&log, "sibling issue");
    root.finish(false);
    let child_text = text(child.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(child_text.contains("child issue"));
    assert!(!child_text.contains("root issue"));
    assert!(!child_text.contains("sibling issue"));
    let root_text = text(root.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert_eq!(root_text.matches("child issue").count(), 1);
    assert!(root_text.contains("Observed issues: 3"));
}

#[test]
fn diagnostic_larger_than_one_io_chunk_is_preserved_without_an_artificial_limit() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    let name = "資料".repeat(MAX_IO);
    record(&log, &name);
    report.finish(false);
    let contents = text(report.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(contents.contains(&name));
    assert!(contents.contains("Stored details: 1"));
}

#[test]
fn unavailable_report_storage_keeps_observing_and_reports_missing_details() {
    let temporary = tempfile::tempdir().unwrap();
    let unavailable = temporary.path().join("occupied");
    std::fs::write(&unavailable, b"not a directory").unwrap();
    let log = ReportLog::new(&ReportArena::testing(), unavailable);
    let report = log.mark().report();
    for name in ["a", "b", "c"] {
        record(&log, name);
    }
    report.finish(true);
    let contents = text(report.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(contents.contains("Observed issues: 3"));
    assert!(contents.contains("Stored details: 0"));
    assert!(contents.contains("Report storage failed"));
    assert!(contents.contains("Counting continued"));
    assert!(contents.contains("Scan cancelled"));
}

#[test]
fn cancelled_snapshot_request_never_opens_a_report_source() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    record(&log, "a");
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        matches!(report.snapshot(&cancel), Err(error) if error.kind == io::ErrorKind::Interrupted)
    );
}

struct StorageFull {
    spool: AppendSpool,
    remaining: usize,
}
impl Storage for StorageFull {
    fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<u64> {
        if bytes.len() > self.remaining {
            return Err(io::ErrorKind::StorageFull.into());
        }
        self.remaining -= bytes.len();
        self.spool.append(offset, bytes)
    }
    fn length(&self) -> u64 {
        self.spool.length()
    }
    fn prefix(&self, start: u64, end: u64) -> io::Result<SpoolPrefix> {
        self.spool.prefix(start, end)
    }
}

#[test]
fn storage_failure_in_a_multichunk_record_never_exposes_its_incomplete_text() {
    let (temporary, log) = log();
    let reservation = log.0.lock().unwrap().arena.reserve().unwrap();
    let spool = PrivateSpoolDirectory::open(&temporary.path().join("private"))
        .unwrap()
        .create_append_only()
        .unwrap();
    log.0.lock().unwrap().storage = Some(reservation.retain(Mutex::new(Box::new(StorageFull {
        spool,
        remaining: BUFFER_BYTES + 1_024,
    })
        as Box<dyn Storage>)));
    let report = log.mark().report();
    record(&log, "saved before storage failure");
    record(
        &log,
        &format!("unfinished {}", "x".repeat(BUFFER_BYTES * 3)),
    );
    record(&log, "observed after storage failure");
    report.finish(false);
    let contents = text(report.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(contents.contains("saved before storage failure"));
    assert!(!contents.contains("unfinished"));
    assert!(!contents.contains("observed after storage failure"));
    assert!(contents.contains("Observed issues: 3"));
    assert!(contents.contains("Stored details: 1"));
    assert!(contents.contains("Report storage failed"));
}

#[test]
fn completed_subtree_does_not_inherit_a_later_siblings_storage_failure() {
    let (temporary, log) = log();
    let reservation = log.0.lock().unwrap().arena.reserve().unwrap();
    let spool = PrivateSpoolDirectory::open(&temporary.path().join("private"))
        .unwrap()
        .create_append_only()
        .unwrap();
    log.0.lock().unwrap().storage = Some(reservation.retain(Mutex::new(Box::new(StorageFull {
        spool,
        remaining: 1_024,
    })
        as Box<dyn Storage>)));
    let child = log.mark().report();
    record(&log, "completed child issue");
    child.finish(false);
    record(&log, &"later sibling".repeat(BUFFER_BYTES));
    let contents = text(child.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(contents.contains("Observed issues: 1"));
    assert!(contents.contains("Stored details: 1"));
    assert!(!contents.contains("Report storage failed"));
}

#[test]
fn cancellation_alone_is_not_a_warning() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    report.finish(true);
    let contents = text(report.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(contents.contains("Observed issues: 0"));
    assert!(contents.contains("Stored details: 0"));
    assert!(contents.contains("Scan cancelled"));
    assert!(!contents.contains("Path:"));
    assert!(!contents.contains("Report storage failed"));
}

#[test]
fn control_characters_in_provider_reasons_do_not_make_binary_or_forged_records() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    log.record(
        format_args!("{:?}", Path::new("actual")),
        "read\rdirectory",
        io::ErrorKind::PermissionDenied,
        "Denied\0\nPath: forged\u{1b}[1m",
    );
    report.finish(false);
    let contents = text(report.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(!contents.contains('\0'));
    assert!(!contents.contains('\u{1b}'));
    assert!(!contents.contains('\r'));
    assert_eq!(
        contents
            .lines()
            .filter(|line| line.starts_with("Path:"))
            .count(),
        1
    );
    assert!(contents.contains("forged"));
}

#[test]
fn concurrent_snapshot_counts_only_complete_records_and_keeps_its_own_length() {
    let (_temporary, log) = log();
    let report = log.mark().report();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for index in 0..128 {
                record(&log, &format!("directory {index}"));
            }
            report.finish(false);
        });
        for _ in 0..128 {
            let source = report.snapshot(&Cancellation::default()).unwrap();
            let first = text(source.as_ref());
            let count = first
                .lines()
                .find_map(|line| line.strip_prefix("Observed issues: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            assert_eq!(
                first
                    .lines()
                    .filter(|line| line.starts_with("Path:"))
                    .count(),
                count
            );
            assert_eq!(first, text(source.as_ref()));
        }
    });
}

#[test]
fn a_failed_report_does_not_poison_another_file_in_the_same_arena() {
    let temporary = tempfile::tempdir().unwrap();
    let occupied = temporary.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let arena = ReportArena::testing();
    let bad = ReportLog::new(&arena, occupied);
    let good = ReportLog::new(&arena, temporary.path().join("good"));
    let bad_range = bad.mark().report();
    let good_range = good.mark().report();
    record(&bad, "first unavailable details");
    record(&good, "first complete 東京 record");
    record(&bad, "second unavailable details");
    record(&good, "second complete 資料 record");
    bad_range.finish(false);
    good_range.finish(false);
    let bad_text = text(
        bad_range
            .snapshot(&Cancellation::default())
            .unwrap()
            .as_ref(),
    );
    assert!(bad_text.contains("Observed issues: 2"));
    assert!(bad_text.contains("Stored details: 0"));
    assert!(bad_text.contains("Report storage failed"));
    let good_text = text(
        good_range
            .snapshot(&Cancellation::default())
            .unwrap()
            .as_ref(),
    );
    assert!(good_text.contains("first complete 東京 record"));
    assert!(good_text.contains("second complete 資料 record"));
    assert!(good_text.contains("Stored details: 2"));
    assert!(!good_text.contains("Report storage failed"));
}

fn shared(path: PathBuf) -> SpoolRoot {
    SpoolRoot {
        path,
        per_process: false,
    }
}

fn per_process(path: PathBuf) -> SpoolRoot {
    SpoolRoot {
        path,
        per_process: true,
    }
}

#[test]
fn spool_roots_prefer_the_runtime_directory_then_per_user_then_per_process_names() {
    let temporary = std::env::temp_dir();
    let runtime = temporary.join("runtime");
    let roots = spool_roots(
        Some(runtime.clone().into()),
        Some("alice/../x".into()),
        temporary.clone(),
        42,
        "size-warnings",
    );
    assert_eq!(
        roots,
        [
            shared(runtime.join("MidnightExplorer").join("size-warnings")),
            shared(temporary.join("MidnightExplorer-616c6963652f2e2e2f78-size-warnings")),
            per_process(temporary.join("MidnightExplorer-size-warnings-42")),
        ]
    );
    // A relative runtime directory and an empty account name are ignored.
    let roots = spool_roots(
        Some("relative".into()),
        Some("".into()),
        temporary.clone(),
        7,
        "size-warnings",
    );
    assert_eq!(
        roots,
        [per_process(
            temporary.join("MidnightExplorer-size-warnings-7")
        )]
    );
}

#[test]
fn per_user_spool_roots_keep_non_ascii_account_names_distinct() {
    let temporary = std::env::temp_dir();
    let per_user = |user: &str| {
        let roots = spool_roots(
            None,
            Some(user.into()),
            temporary.clone(),
            7,
            "size-warnings",
        );
        assert_eq!(roots.len(), 2, "{user} keeps a per-user root");
        assert!(!roots[0].per_process && roots[1].per_process);
        roots[0].path.clone()
    };
    assert_eq!(
        per_user("Пользователь"),
        temporary.join(
            "MidnightExplorer-d09fd0bed0bbd18cd0b7d0bed0b2d0b0d182d0b5d0bbd18c-size-warnings"
        )
    );
    assert_eq!(
        per_user("山田"),
        temporary.join("MidnightExplorer-e5b1b1e794b0-size-warnings")
    );
    // Names an ASCII filter would have merged, or dropped entirely, stay apart.
    let names = [
        "Пользователь",
        "Пользователи",
        "山田",
        "山本",
        "jöe",
        "je",
        "Bob",
        "bob",
    ];
    let roots = names
        .iter()
        .map(|name| per_user(name))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(roots.len(), names.len());
    // A name too long for one path component keeps only the per-process root.
    let roots = spool_roots(
        None,
        Some("山".repeat(40).into()),
        temporary.clone(),
        7,
        "size-warnings",
    );
    assert_eq!(
        roots,
        [per_process(
            temporary.join("MidnightExplorer-size-warnings-7")
        )]
    );
}

#[test]
fn a_per_process_root_is_removed_after_its_last_spool_closes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("MidnightExplorer-size-warnings-7");
    let roots = [per_process(path.clone())];
    let first = create_private_spool(&roots, PrivateSpoolDirectory::create_append_only).unwrap();
    let mut second =
        create_private_spool(&roots, PrivateSpoolDirectory::create_append_only).unwrap();
    assert!(path.is_dir());
    drop(first);
    // The other spool still uses the root: it stays and accepts new spools.
    assert!(path.is_dir());
    second.append(0, b"kept").unwrap();
    let third = create_private_spool(&roots, PrivateSpoolDirectory::create_append_only).unwrap();
    drop((second, third));
    assert!(
        !path.exists(),
        "the last spool removes its per-process root"
    );
    // A shared or per-user root is kept for later runs.
    let kept = temporary.path().join("per-user");
    drop(
        create_private_spool(
            &[shared(kept.clone())],
            PrivateSpoolDirectory::create_append_only,
        )
        .unwrap(),
    );
    assert!(kept.is_dir());
}

#[test]
fn a_per_process_report_root_is_removed_after_its_last_log_and_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("MidnightExplorer-size-warnings-8");
    let arena = ReportArena::testing();
    let log = ReportLog::with_candidates(&arena, vec![per_process(path.clone())], DEFAULT_TITLE);
    let range = log.mark().report();
    record(&log, "kept 資料 record");
    range.finish(false);
    let snapshot = range.snapshot(&Cancellation::default()).unwrap();
    drop((log, range));
    // The snapshot keeps the storage, its root and the record readable.
    assert!(path.is_dir());
    assert!(text(snapshot.as_ref()).contains("kept 資料 record"));
    drop(snapshot);
    // The report cleanup worker closes the storage and then removes the root.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the per-process root outlived its last report"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn an_unusable_shared_root_falls_back_to_the_next_private_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    // Stands in for a shared root that another account created first.
    let occupied = temporary.path().join("occupied");
    std::fs::write(&occupied, b"not a private directory").unwrap();
    let log = ReportLog::with_candidates(
        &ReportArena::testing(),
        vec![shared(occupied), shared(temporary.path().join("private"))],
        DEFAULT_TITLE,
    );
    let range = log.mark().report();
    record(&log, "kept 資料 record");
    range.finish(false);
    let report = text(range.snapshot(&Cancellation::default()).unwrap().as_ref());
    assert!(report.contains("kept 資料 record"));
    assert!(report.contains("Stored details: 1"));
    assert!(!report.contains("Report storage failed"));
}
