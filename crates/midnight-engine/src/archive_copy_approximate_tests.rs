//! A part of the source that cannot be counted leaves the totals approximate instead of
//! unknown: the bar and the estimate stay, and the totals grow with what the copy finds.
use super::*;

/// `tree()` of the progress tests, whose `folder/sub` cannot be listed the first time.
#[derive(Debug)]
struct FirstListingFails {
    inner: Arc<MemoryMount>,
    folder: u64,
    failed: AtomicBool,
}
impl ArchiveMount for FirstListingFails {
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
        if directory == self.folder && !self.failed.swap(true, Ordering::AcqRel) {
            return Err(error(
                Path::new("sub"),
                io::ErrorKind::PermissionDenied,
                "Access is denied.",
            ));
        }
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
fn a_folder_that_cannot_be_counted_leaves_an_approximate_total_that_grows_with_the_copy() {
    let temp = fixture();
    let inner = mount(vec![
        node(2, 1, b"folder", EntryKind::Directory),
        node(3, 2, b"child.txt", EntryKind::File),
        node(4, 2, b"sub", EntryKind::Directory),
        node(5, 4, b"deep.txt", EntryKind::File),
        node(6, 1, b"top.txt", EntryKind::File),
    ]);
    let mut request = input(inner.clone(), temp.path());
    request.location.mount = Some(Arc::new(FirstListingFails {
        inner,
        folder: 4,
        failed: AtomicBool::new(false),
    }));
    let cancel = Cancellation::default();
    let roots = archive_source::resolve(&request, &cancel).unwrap();
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare_archive(temp.path().into(), roots, &service, cancel, status.clone())
            .unwrap();
    job.prepare_discovery();
    let counted = snapshot(&status);
    // `folder`, `child.txt`, `sub` and the next selected item, `top.txt`: what could be
    // counted, kept as an approximation rather than given up.
    assert_eq!(
        counted.discovery.state,
        DiscoveryState::Complete,
        "{counted:?}"
    );
    assert!(counted.discovery.approximate, "{counted:?}");
    assert_eq!(
        (
            counted.discovery.files,
            counted.discovery.directories,
            counted.discovery.bytes
        ),
        (2, 2, Some(14))
    );
    assert_eq!(
        counted.remaining_work.map(|work| work.write_bytes),
        Some(14)
    );
    for _ in 0..200 {
        let step = job.step();
        let progress = snapshot(&status);
        // The totals grow with the copy, so that what is done never passes them.
        if let Some(bytes) = progress.discovery.bytes {
            assert!(
                progress.completed_bytes + progress.current_file_copied <= bytes,
                "{progress:?}"
            );
        }
        assert!(progress.files_completed <= progress.discovery.files);
        match step {
            worker::Step::Continue => {}
            worker::Step::Done => {
                let done = job.finish();
                assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
                assert_eq!(done.files_completed, 3);
                assert_eq!(done.discovery.state, DiscoveryState::Complete);
                assert!(done.discovery.approximate);
                assert_eq!((done.discovery.files, done.discovery.bytes), (3, Some(21)));
                assert_eq!(
                    fs::read(temp.path().join("folder/sub/deep.txt")).unwrap(),
                    b"payload"
                );
                return;
            }
            _ => panic!("unexpected Copy decision: {progress:?}"),
        }
    }
    panic!("archive copy did not complete");
}
