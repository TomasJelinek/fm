//! Find launch boundaries: display paths are not authority and stale captures never publish.
use crate::{NativeCapture, capture_find};
use midnight_core::*;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
struct Source {
    entry: FindEntry,
    checks: AtomicUsize,
    valid: bool,
}
impl CapturedFindFile for Source {
    fn request(&self) -> FindValidationRequest {
        FindValidationRequest {
            session: FindSessionId(1),
            generation: FindGeneration(1),
            result: FindResultId(1),
            token: 1,
        }
    }
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn validate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.checks.fetch_add(1, Ordering::Relaxed);
        if !self.valid || cancel.is_cancelled() {
            return Err(find_error(
                &self.entry.path,
                std::io::ErrorKind::Interrupted,
                "revoked Find",
            ));
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Foreign;
impl ContentSource for Foreign {
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "foreign.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        b"unknown-provider-version".to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        panic!("foreign source must never be read")
    }
}
fn fixture(valid: bool, kind: EntryKind) -> (ViewerLaunch, Arc<Source>) {
    let source = Arc::new(Source {
        valid,
        checks: AtomicUsize::new(0),
        entry: FindEntry {
            path: PathBuf::from("retained-native-path"),
            kind,
            metadata: Default::default(),
            identity: TransferIdentity {
                device: 1,
                object: [2; 16],
            },
            revision: FindRevision([3; 6]),
        },
    });
    (
        ViewerLaunch {
            find: Some(source.clone()),
            associations: Arc::default(),
            generated: None,
            preferences: Arc::default(),
            archive: None,
            tree: None,
            entries: Arc::from([]),
            view: Arc::new(DirectoryIndex::from_entries(&[])),
            focused: "display-is-not-authority".into(),
            basic: true,
            disabled_plugins: Arc::from([]),
        },
        source,
    )
}
#[test]
fn closed_or_revoked_find_never_calls_native_capture() {
    for (valid, closed) in [(true, true), (false, false)] {
        let (launch, _) = fixture(valid, EntryKind::File);
        let callback: NativeCapture = Arc::new(|_, _| panic!("no native admission"));
        assert!(
            capture_find::prepare(&launch, &Arc::new(AtomicBool::new(closed)), callback).is_err()
        );
    }
}
#[test]
fn folder_find_results_do_not_enter_file_viewer() {
    let (launch, source) = fixture(true, EntryKind::Directory);
    let callback: NativeCapture = Arc::new(|_, _| panic!("folder capture not allowed"));
    assert!(capture_find::prepare(&launch, &Arc::default(), callback).is_err());
    assert_eq!(source.checks.load(Ordering::Relaxed), 0);
}
#[test]
fn native_capture_uses_retained_path_and_rejects_unknown_content_identity() {
    let (launch, source) = fixture(true, EntryKind::File);
    let expected = source.entry.path.clone();
    let callback: NativeCapture = Arc::new(move |path, _| {
        assert_eq!(path, expected);
        CapturedFile::from_source(Arc::new(Foreign)).map_err(|error| error.to_string())
    });
    let error = capture_find::prepare(&launch, &Arc::default(), callback)
        .err()
        .unwrap();
    assert!(error.contains("identity"));
    assert_eq!(source.checks.load(Ordering::Relaxed), 1);
}
#[test]
fn cancellation_observes_external_close_without_cancel_authority() {
    let external = Arc::new(AtomicBool::new(false));
    let token = Cancellation::observing(external.clone());
    let second = Cancellation::observing(external.clone());
    token.cancel();
    assert!(!external.load(Ordering::Acquire));
    assert!(!second.is_cancelled());
    external.store(true, Ordering::Release);
    assert!(second.is_cancelled());
}
