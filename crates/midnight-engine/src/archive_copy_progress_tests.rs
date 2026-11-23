//! Copying out of an archive shows what normal Copy shows: the path of the member being
//! copied (the archive file's path, the folders inside it and the member's name), the
//! number of files and bytes before the first one is written, and so a determinate bar
//! and an estimate.
use super::*;

/// `top.txt`, and `folder` holding `child.txt` and `sub/deep.txt`: three files of
/// seven bytes in two folders.
fn tree() -> Arc<MemoryMount> {
    mount(vec![
        node(2, 1, b"folder", EntryKind::Directory),
        node(3, 2, b"child.txt", EntryKind::File),
        node(4, 2, b"sub", EntryKind::Directory),
        node(5, 4, b"deep.txt", EntryKind::File),
        node(6, 1, b"top.txt", EntryKind::File),
    ])
}

#[test]
fn an_archive_copy_counts_its_members_first_and_names_each_by_its_archive_path() {
    let temp = fixture();
    let input = input(tree(), temp.path());
    let archive = temp.path().join("source.zip");
    let cancel = Cancellation::default();
    let roots = archive_source::resolve(&input, &cancel).unwrap();
    let status = Arc::new(Mutex::new(initial(&input.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare_archive(temp.path().into(), roots, &service, cancel, status.clone())
            .unwrap();
    job.prepare_discovery();
    let counted = snapshot(&status);
    assert_eq!(
        counted.discovery.state,
        DiscoveryState::Complete,
        "{counted:?}"
    );
    assert_eq!(counted.discovery.scope, DiscoveryScope::Recursive);
    assert_eq!(
        (
            counted.discovery.files,
            counted.discovery.directories,
            counted.discovery.bytes
        ),
        (3, 2, Some(21))
    );
    let work = counted
        .remaining_work
        .expect("the counted bytes give the estimate its budget");
    assert_eq!(work.write_bytes, 21);
    let mut shown = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let step = job.step();
        let progress = snapshot(&status);
        if let Some(path) = progress.current_path.clone() {
            assert!(
                path.starts_with(&archive),
                "{} lies outside {}",
                path.display(),
                archive.display()
            );
            shown.insert(path);
        }
        match step {
            worker::Step::Continue => {}
            worker::Step::Done => {
                let done = job.finish();
                assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
                // The count matched the copy, so it stayed a valid denominator.
                assert_eq!(done.discovery.state, DiscoveryState::Complete, "{done:?}");
                assert_eq!(done.files_completed, 3);
                assert_eq!(done.remaining_work, Some(TransferRemainingWork::default()));
                for member in ["top.txt", "folder/child.txt", "folder/sub/deep.txt"] {
                    assert!(
                        shown.contains(&archive.join(member)),
                        "{member} was never shown by its path: {shown:?}"
                    );
                    assert_eq!(fs::read(temp.path().join(member)).unwrap(), b"payload");
                }
                return;
            }
            _ => panic!("unexpected Copy decision: {progress:?}"),
        }
    }
    panic!("archive copy did not complete");
}

/// The folders of `tree()`, counting each listing.
#[derive(Debug)]
struct Listings {
    inner: Arc<MemoryMount>,
    listed: AtomicUsize,
}
impl ArchiveMount for Listings {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(owner) = Arc::into_inner(self) {
            drop(owner);
        }
        Ok(())
    }
    fn id(&self) -> u64 {
        self.inner.id()
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.listed.fetch_add(1, Ordering::Relaxed);
        self.inner.list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.inner.read(entry, offset, length, cancel)
    }
}

#[test]
fn counting_lists_each_folder_once_and_reads_no_member() {
    let temp = fixture();
    let inner = tree();
    let listings = Arc::new(Listings {
        inner: inner.clone(),
        listed: AtomicUsize::new(0),
    });
    let mut request = input(inner, temp.path());
    request.location.mount = Some(listings.clone());
    let cancel = Cancellation::default();
    let roots = archive_source::resolve(&request, &cancel).unwrap();
    let listed_to_resolve = listings.listed.load(Ordering::Relaxed);
    let mut items = Vec::new();
    for root in &roots {
        root.discover(&cancel, &mut |item| items.push(item.path.clone()))
            .unwrap();
    }
    // `folder` and `folder/sub`; `top.txt` is a file and needs no listing.
    assert_eq!(
        listings.listed.load(Ordering::Relaxed) - listed_to_resolve,
        2
    );
    let archive = temp.path().join("source.zip");
    items.sort();
    let mut expected: Vec<_> = [
        "folder",
        "folder/child.txt",
        "folder/sub",
        "folder/sub/deep.txt",
        "top.txt",
    ]
    .iter()
    .map(|member| archive.join(member))
    .collect();
    expected.sort();
    assert_eq!(items, expected);
}

#[test]
fn a_stopped_count_ends_without_listing_the_rest() {
    let temp = fixture();
    let request = input(tree(), temp.path());
    let roots = archive_source::resolve(&request, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    let error = roots[0].discover(&cancel, &mut |_| {}).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
}
