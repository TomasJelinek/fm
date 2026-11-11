use super::*;

fn fixture(mode: SelectionMode) -> RectangleSummaryRequest {
    let entries: Vec<_> = (0..4)
        .map(|id| {
            let mut entry = Entry::new(
                EntryId(id),
                format!("root/entry{id}").into(),
                if id == 2 {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            );
            entry.metadata.size = Some((id + 1) * 10);
            entry
        })
        .collect();
    let full = Arc::new(DirectoryIndex::from_entries(&entries));
    let view = Arc::new(DirectoryIndex::from_entries(&entries[..3]));
    RectangleSummaryRequest {
        pane: PaneId::Left,
        token: Arc::new(()),
        view,
        seed: RectangleSelectionSeed::Native(Arc::new(
            SelectionSnapshot::from_bits(full, vec![0b1001]).unwrap(),
        )),
        ranges: SelectionRanges::new(3, vec![SelectionSpan { first: 0, last: 1 }]).unwrap(),
        mode,
        cancel: Cancellation::default(),
    }
}
#[test]
fn rectangle_totals_match_modifiers_including_hidden_existing_marks() {
    for (mode, count, bytes) in [
        (SelectionMode::Replace, 2, 30),
        (SelectionMode::Add, 3, 70),
        (SelectionMode::Toggle, 2, 60),
    ] {
        let request = fixture(mode);
        let totals = summarize(&request, &Cancellation::default()).unwrap();
        assert_eq!(
            (totals.files(), totals.folders(), totals.file_bytes()),
            (count, 0, Some(bytes))
        );
        let RectangleSelectionSeed::Native(seed) = &request.seed else {
            unreachable!()
        };
        assert_eq!(
            (seed.len(), seed.file_bytes()),
            (2, Some(50)),
            "preview cannot commit selection"
        );
    }
}
#[test]
fn rectangle_totals_count_folders_and_preserve_unknown_lengths() {
    let mut request = fixture(SelectionMode::Replace);
    request.ranges = SelectionRanges::new(3, vec![SelectionSpan { first: 1, last: 2 }]).unwrap();
    let totals = summarize(&request, &Cancellation::default()).unwrap();
    assert_eq!(
        (totals.files(), totals.folders(), totals.file_bytes()),
        (1, 1, Some(20))
    );
    let RectangleSelectionSeed::Native(seed) = &request.seed else {
        unreachable!()
    };
    let mut basis = (**seed.basis()).clone();
    basis.file_sizes[1] = None;
    Arc::make_mut(&mut request.view).file_sizes[1] = None;
    request.seed = RectangleSelectionSeed::Native(Arc::new(
        SelectionSnapshot::from_bits(Arc::new(basis), vec![0]).unwrap(),
    ));
    assert_eq!(
        summarize(&request, &Cancellation::default())
            .unwrap()
            .file_bytes(),
        None
    );
}
#[test]
fn archive_rectangle_uses_virtual_ids_not_native_paths() {
    let mut request = fixture(SelectionMode::Toggle);
    request.seed = RectangleSelectionSeed::Archive {
        selected: Arc::new([EntryId(0)].into()),
        full: request.view.clone(),
    };
    let totals = summarize(&request, &Cancellation::default()).unwrap();
    assert_eq!(
        (totals.files(), totals.folders(), totals.file_bytes()),
        (1, 0, Some(20))
    );
}
#[test]
fn obsolete_or_cancelled_rectangle_never_publishes_totals() {
    let mut request = fixture(SelectionMode::Replace);
    request.ranges = SelectionRanges::new(2, vec![]).unwrap();
    assert!(summarize(&request, &Cancellation::default()).is_none());
    request = fixture(SelectionMode::Replace);
    request.cancel.cancel();
    assert!(summarize(&request, &Cancellation::default()).is_none());
    let stop = Cancellation::default();
    stop.cancel();
    assert!(summarize(&fixture(SelectionMode::Replace), &stop).is_none());
}
#[tokio::test]
async fn latest_rectangle_coalesces_and_releases_captured_indices() {
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let mut last = None;
    for _ in 0..50 {
        let request = Arc::new(fixture(SelectionMode::Add));
        controller.submit(request.clone());
        last = Some(request);
    }
    let last = last.unwrap();
    let index = Arc::downgrade(&last.view);
    let token = last.token.clone();
    drop(last);
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, Event::RectangleSummary { token: actual, .. } if Arc::ptr_eq(&actual, &token))
    );
    drop(controller);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(
        index.upgrade().is_none(),
        "idle summaries must release the captured listing"
    );
}

#[tokio::test]
async fn replacement_between_wake_and_take_cannot_discard_latest_result() {
    let requests = std::array::from_fn(|_| (Arc::default(), watch::channel(()).0));
    let controller = Controller {
        requests,
        stop: Cancellation::default(),
    };
    let mut receiver = controller.requests[0].1.subscribe();
    controller.submit(Arc::new(fixture(SelectionMode::Replace)));
    receiver.changed().await.unwrap(); // Wake A is consumed, mailbox not taken yet.
    let replacement = Arc::new(fixture(SelectionMode::Toggle));
    controller.submit(replacement.clone()); // Wake B precedes worker's take.
    let taken = take_latest(&controller.requests[0].0, &mut receiver).unwrap();
    assert!(Arc::ptr_eq(&taken, &replacement));
    assert!(
        !receiver.has_changed().unwrap(),
        "B's own wake must not discard B's result"
    );
    assert!(controller.requests[0].0.lock().unwrap().is_none());
}
