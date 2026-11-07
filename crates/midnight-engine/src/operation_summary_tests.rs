use super::*;

struct SummaryService {
    items: Vec<MutationItem>,
    calls: Arc<Calls>,
}

impl MutationService for SummaryService {
    fn prepare(&self, path: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        self.calls.prepared.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(FakeTarget {
            item: self
                .items
                .iter()
                .find(|item| item.path == path)
                .unwrap()
                .clone(),
            calls: self.calls.clone(),
        }))
    }
}

fn prepared_summary(kinds_and_sizes: &[(EntryKind, Option<u64>)]) -> OperationPreview {
    let root = std::env::temp_dir().join("delete-summary-fixture");
    let items: Vec<_> = kinds_and_sizes
        .iter()
        .enumerate()
        .map(|(index, (kind, size))| MutationItem {
            path: root.join(format!("文档-{index}")),
            kind: *kind,
            metadata: EntryMetadata {
                size: *size,
                ..Default::default()
            },
        })
        .collect();
    // Listing metadata is deliberately older than the pinned native preparation.
    let entries: Arc<[Entry]> = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let mut entry = Entry::new(EntryId(index as u64), item.path.clone(), EntryKind::File);
            entry.metadata.size = Some(1);
            entry
        })
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    assert!(entries.len() < 64);
    let selection = Arc::new(
        SelectionSnapshot::from_bits(view.clone(), vec![(1u64 << entries.len()) - 1]).unwrap(),
    );
    let request = PrepareOperation {
        id: OperationId(42),
        kind: MutationKind::Delete(DeleteMode::Permanent),
        source: VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(7),
            path: root,
            entries,
        },
        view,
        selection,
        focused: None,
    };
    let calls = Arc::new(Calls::default());
    let job = worker::prepare(
        request.clone(),
        Some(Arc::new(SummaryService {
            items,
            calls: calls.clone(),
        })),
        Cancellation::default(),
        Arc::new(Mutex::new(worker::initial(&request))),
        Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
    )
    .unwrap();
    let preview = job.preview();
    assert_eq!(
        calls.prepared.load(Ordering::Relaxed),
        kinds_and_sizes.len()
    );
    assert!(
        calls.deleted.lock().unwrap().is_empty(),
        "preparation must not delete"
    );
    preview
}

#[test]
fn delete_summary_uses_all_prepared_file_lengths_beyond_the_preview_sample() {
    let mut items = vec![(EntryKind::File, Some(100)); 40];
    items.push((EntryKind::Directory, Some(u64::MAX)));
    items.push((EntryKind::DirectoryLink, Some(u64::MAX)));
    let preview = prepared_summary(&items);
    assert_eq!((preview.files, preview.folders), (40, 2));
    assert_eq!(preview.sample.len(), 32);
    assert_eq!(preview.file_bytes, Some(4_000));
}

#[test]
fn delete_summary_never_presents_unknown_or_overflowed_file_bytes_as_exact() {
    for items in [
        vec![
            (EntryKind::File, Some(u64::MAX)),
            (EntryKind::File, Some(1)),
        ],
        vec![(EntryKind::File, Some(1)), (EntryKind::File, None)],
        vec![(EntryKind::Symlink, Some(1))],
        vec![(EntryKind::Other, Some(1))],
    ] {
        assert_eq!(prepared_summary(&items).file_bytes, None, "{items:?}");
    }
    assert_eq!(
        prepared_summary(&[(EntryKind::File, Some(u64::MAX))]).file_bytes,
        Some(u64::MAX)
    );
    assert_eq!(
        prepared_summary(&[(EntryKind::File, Some(0))]).file_bytes,
        Some(0)
    );
    assert_eq!(
        prepared_summary(&[(EntryKind::Directory, None)]).file_bytes,
        Some(0)
    );
}
