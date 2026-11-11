use super::*;
use std::cell::Cell;

fn progress(id: u64) -> Arc<CopyProgress> {
    Arc::new(CopyProgress::queued(
        OperationId(id),
        "target".into(),
        1,
        midnight_core::TransferIntent::Copy,
    ))
}
fn row(builds: &Cell<usize>) -> OperationRow {
    builds.set(builds.get() + 1);
    OperationRow {
        detail: builds.get().to_string().into(),
        ..Default::default()
    }
}

#[test]
fn exact_snapshots_reuse_rows_and_optimistic_mutation_or_locale_rebuilds() {
    let cache = Rows::default();
    let builds = Cell::new(0);
    let mut source = progress(1);
    let mut locale = NumberLocale::default();
    let first = cache.get(Snapshot::Copy(&source, None), &locale, || row(&builds));
    let reused = cache.get(Snapshot::Copy(&source, None), &locale, || row(&builds));
    // Slint Image(None) is not reflexively equal. Check the text and its shared
    // allocation instead of treating an OperationRow as a whole-row value oracle.
    assert_eq!(reused.detail, first.detail);
    assert!(std::ptr::eq(
        reused.detail.as_str().as_ptr(),
        first.detail.as_str().as_ptr()
    ));
    assert_eq!(builds.get(), 1);
    Arc::make_mut(&mut source).pause_requested = true;
    cache.get(Snapshot::Copy(&source, None), &locale, || row(&builds));
    assert_eq!(
        builds.get(),
        2,
        "optimistic pause replaces the retained Arc"
    );
    locale.group_separator = " ".into();
    cache.get(Snapshot::Copy(&source, None), &locale, || row(&builds));
    assert_eq!(builds.get(), 3);
    source = Arc::new((*source).clone());
    cache.get(Snapshot::Copy(&source, None), &locale, || row(&builds));
    assert_eq!(builds.get(), 4, "equal contents are not an exact snapshot");
}

#[test]
fn cache_does_not_retain_core_payloads_and_clear_removes_derived_rows() {
    let cache = Rows::default();
    let source = progress(7);
    let weak = Arc::downgrade(&source);
    cache.get(
        Snapshot::Copy(&source, None),
        &NumberLocale::default(),
        OperationRow::default,
    );
    cache.retain([Key::Copy(OperationId(7))].into_iter());
    assert_eq!(cache.0.borrow().len(), 1);
    drop(source);
    assert!(
        weak.upgrade().is_none(),
        "derived cache must not retain retired core payloads"
    );
    cache.retain(std::iter::empty());
    assert!(cache.0.borrow().is_empty());
}

#[test]
fn projection_refreshes_pause_locale_failures_attention_and_async_artwork() {
    use crate::{Presenter, job_presentation};
    use midnight_core::*;
    let mut p = Presenter::new("left".into(), "right".into());
    let mut value = (*progress(1)).clone();
    value.state = CopyOperationState::Running;
    value.copied_bytes = 1234;
    p.copies.progress.push(Arc::new(value));
    let first = job_presentation::rows(&p).remove(0);
    assert!(!first.attention && !first.pause_requested && !first.failed);
    assert_eq!(first.icon.size().width, 0);
    p.apply(Event::IconLoaded {
        key: IconKey::action(ActionIcon::Copy),
        pixels: Arc::new(IconPixels::new(1, 1, vec![20, 40, 60, 255]).unwrap()),
    });
    let item = MutationItem {
        path: "source".into(),
        kind: EntryKind::File,
        metadata: Default::default(),
    };
    p.copies.conflicts.push(Arc::new(CopyConflict {
        id: OperationId(1),
        version: 1,
        source: item.clone(),
        target: item,
        destination: "target".into(),
        can_replace: true,
        can_merge: false,
    }));
    let row = job_presentation::rows(&p).remove(0);
    assert!(row.attention);
    assert_eq!(
        row.icon.size().width,
        1,
        "a cached row accepts later native artwork"
    );
    p.number_locale = NumberLocale {
        group_separator: "_".into(),
        grouping: vec![3],
        repeat_last_group: true,
        ..Default::default()
    };
    assert!(
        job_presentation::rows(&p)[0]
            .detail
            .contains("1_234 bytes copied")
    );
    p.set_job_paused("copy-job:1", true, &|command| {
        matches!(
            command,
            Command::SetCopyPaused {
                id: OperationId(1),
                paused: true
            }
        )
    });
    assert!(job_presentation::rows(&p)[0].pause_requested);
    p.copies.failures.push(Arc::new(CopyFailure {
        id: OperationId(1),
        version: 2,
        error: FsError::from_io(
            "write",
            std::path::Path::new("target"),
            std::io::Error::other("late diagnostic"),
        ),
        source: Some("source".into()),
        destination: "target".into(),
        stage: CopyErrorStage::Writing,
        can_retry: true,
        can_skip: true,
    }));
    let row = job_presentation::rows(&p).remove(0);
    assert!(row.failed && row.detail.contains("late diagnostic"));
    p.copies.failures.clear();
    p.copies.conflicts.clear();
    let row = job_presentation::rows(&p).remove(0);
    assert!(!row.attention && !row.failed);
    assert!(!row.detail.contains("late diagnostic"));
}

#[test]
fn late_metadata_results_and_clear_invalidate_derived_history_without_retaining_results() {
    use crate::{Presenter, job_presentation};
    use midnight_core::*;
    let mut p = Presenter::new("left".into(), "right".into());
    p.operations.progress.push(Arc::new(OperationProgress {
        id: OperationId(2),
        kind: MutationKind::EditMetadata,
        state: OperationState::Completed,
        pause_requested: false,
        paused: false,
        total: 2,
        completed: 2,
        failed: 0,
        processed_entries: 2,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
        throughput: Arc::default(),
        discovery: Default::default(),
    }));
    assert!(!job_presentation::rows(&p)[0].detail.contains("examined"));
    p.operations.metadata_results.push(Arc::new(MetadataResult {
        id: OperationId(2),
        processed: 2,
        changed: 1,
        skipped: 1,
        failed: 0,
        sample: vec![],
    }));
    assert!(
        job_presentation::rows(&p)[0]
            .detail
            .contains("2 examined · 1 changed")
    );
    Arc::make_mut(&mut p.operations.metadata_results[0]).changed = 2;
    assert!(
        job_presentation::rows(&p)[0]
            .detail
            .contains("2 examined · 2 changed")
    );
    p.operations
        .metadata_conflicts
        .push(Arc::new(MetadataConflict {
            id: OperationId(2),
            token: 1,
            source: "source".into(),
            destination: "target".into(),
        }));
    assert!(job_presentation::rows(&p)[0].attention);
    let result = Arc::downgrade(&p.operations.metadata_results[0]);
    p.operations.metadata_results.clear();
    assert!(
        result.upgrade().is_none(),
        "cache cannot extend metadata result retention"
    );
    assert!(!job_presentation::rows(&p)[0].detail.contains("examined"));
    p.clear_finished_jobs(false);
    assert!(job_presentation::rows(&p).is_empty());
    assert!(p.job_rows.0.borrow().is_empty());
}
