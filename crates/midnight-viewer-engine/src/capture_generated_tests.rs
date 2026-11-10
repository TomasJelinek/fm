use super::*;
use midnight_core::*;
use midnight_plugin_host::CapturedFile;
use std::{
    io,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Debug)]
enum Fault {
    None,
    EarlyEof,
    ExcessCount,
    ChangeLength,
    Close(Arc<AtomicBool>),
}

#[derive(Debug)]
struct Source {
    bytes: Vec<u8>,
    length: AtomicU64,
    max_chunk: usize,
    fault: Fault,
    calls: Mutex<Vec<(u64, usize)>>,
}

impl Source {
    fn new(bytes: &[u8], max_chunk: usize, fault: Fault) -> Arc<Self> {
        Arc::new(Self {
            bytes: bytes.to_owned(),
            length: AtomicU64::new(bytes.len() as u64),
            max_chunk,
            fault,
            calls: Mutex::new(Vec::new()),
        })
    }
}

impl GeneratedTextSource for Source {
    fn length(&self) -> u64 {
        self.length.load(Ordering::Acquire)
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.calls.lock().unwrap().push((offset, output.len()));
        match &self.fault {
            Fault::EarlyEof => return Ok(0),
            Fault::ExcessCount => return Ok(output.len() + 1),
            Fault::ChangeLength => {
                self.length.fetch_add(1, Ordering::AcqRel);
            }
            Fault::Close(closed) => closed.store(true, Ordering::Release),
            Fault::None => {}
        }
        let offset = usize::try_from(offset).unwrap();
        let count = output
            .len()
            .min(self.max_chunk)
            .min(self.bytes.len().saturating_sub(offset));
        output[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        Ok(count)
    }
}

fn launch(source: Arc<dyn GeneratedTextSource>) -> ViewerLaunch {
    ViewerLaunch {
        associations: Default::default(),
        tree: None,
        generated: Some(source),
        preferences: Arc::new(Default::default()),
        archive: None,
        entries: Arc::from([]),
        view: Arc::new(DirectoryIndex::default()),
        focused: PathBuf::from("never-open-this-generated-label.txt"),
        basic: true,
        disabled_plugins: Arc::from([]),
    }
}

#[test]
fn generated_diagnostic_uses_its_own_title_without_opening_a_native_path() {
    let mut launch = launch(Source::new(b"Reason: Access denied\n", 64, Fault::None));
    launch.focused = PathBuf::from("Size warnings - 資料.txt");
    let mut order = crate::capture::prepare_with(
        &launch,
        &Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| panic!("diagnostic title is not a native path")),
    )
    .unwrap();
    let (name, content) = order.files.remove(0);
    assert_eq!(name, launch.focused);
    let content = content.unwrap().unwrap();
    assert_eq!(content.display_name(), "Size warnings - 資料.txt");
    assert!(content.path().as_os_str().is_empty());
}

fn captured(source: Arc<Source>, closed: &Arc<AtomicBool>) -> CapturedFile {
    let mut order = prepare(&launch(source), closed).unwrap();
    assert_eq!(order.current, 0);
    assert_eq!(order.files.len(), 1);
    assert_eq!(
        order.files[0].0,
        PathBuf::from("never-open-this-generated-label.txt")
    );
    assert!(matches!(
        order.ready.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
    order.files.remove(0).1.unwrap().unwrap()
}

#[test]
fn generated_viewer_reads_exact_unicode_across_short_reads_without_a_native_path() {
    let bytes = "český\r\n文档\r\n🦀".as_bytes();
    let source = Source::new(bytes, 2, Fault::None);
    let closed = Arc::new(AtomicBool::new(false));
    let first = captured(source.clone(), &closed);
    assert!(first.path().as_os_str().is_empty());
    assert_eq!(first.display_name(), "never-open-this-generated-label.txt");
    assert_eq!(first.read_at(0, bytes.len() as u32).unwrap(), bytes);
    assert_eq!(first.read_at(2, 4).unwrap(), &bytes[2..6]);
    assert!(
        source
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, size)| *size <= 256 * 1024)
    );
    assert_eq!(Arc::strong_count(&source), 2);
    drop(first);
    assert_eq!(Arc::strong_count(&source), 1);
}

#[test]
fn generated_capture_dispatch_never_calls_the_native_opener() {
    let source = Source::new(b"file.txt\n", 3, Fault::None);
    let closed = Arc::new(AtomicBool::new(false));
    let order = crate::capture::prepare_with(
        &launch(source),
        &closed,
        Arc::new(|_, _| panic!("generated text must not reopen a label as a path")),
    )
    .unwrap();
    assert_eq!(
        order.files[0]
            .1
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .read_at(0, 9)
            .unwrap(),
        b"file.txt\n"
    );
}

#[test]
fn empty_generated_file_is_valid_and_eof_does_not_read_storage() {
    let source = Source::new(b"", 1, Fault::None);
    let closed = Arc::new(AtomicBool::new(false));
    let first = captured(source.clone(), &closed);
    assert_eq!(first.length(), 0);
    assert!(first.read_at(0, 1).unwrap().is_empty());
    assert!(first.read_at(999, 1).unwrap().is_empty());
    assert!(source.calls.lock().unwrap().is_empty());
}

#[test]
fn early_eof_invalid_counts_and_length_changes_never_return_partial_success() {
    for fault in [Fault::EarlyEof, Fault::ExcessCount, Fault::ChangeLength] {
        let source = Source::new(b"sample", 2, fault);
        let closed = Arc::new(AtomicBool::new(false));
        let first = captured(source.clone(), &closed);
        assert!(first.read_at(0, 6).is_err());
        assert!(source.calls.lock().unwrap().len() <= 3);
    }
}

#[test]
fn closed_viewer_rejects_capture_and_stops_between_short_reads() {
    let closed = Arc::new(AtomicBool::new(true));
    let source = Source::new(b"sample", 2, Fault::None);
    assert!(prepare(&launch(source.clone()), &closed).is_err());
    assert!(source.calls.lock().unwrap().is_empty());
    closed.store(false, Ordering::Release);
    let source = Source::new(b"sample", 2, Fault::Close(closed.clone()));
    let first = captured(source.clone(), &closed);
    assert!(first.read_at(0, 6).is_err());
    assert_eq!(source.calls.lock().unwrap().len(), 1);
    assert!(first.read_at(6, 1).is_err());
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[test]
fn capture_versions_are_distinct_and_invalid_read_bounds_do_not_touch_storage() {
    let source = Source::new(b"sample", 2, Fault::None);
    let closed = Arc::new(AtomicBool::new(false));
    let first = captured(source.clone(), &closed);
    let second = captured(source.clone(), &closed);
    assert_ne!(first.stable_version(), second.stable_version());
    let version = first.stable_version();
    assert_eq!(version, first.stable_version());
    for (offset, length) in [(0, 0), (0, 256 * 1024 + 1), (u64::MAX, 1)] {
        assert!(first.read_at(offset, length).is_err());
    }
    assert!(source.calls.lock().unwrap().is_empty());
}
