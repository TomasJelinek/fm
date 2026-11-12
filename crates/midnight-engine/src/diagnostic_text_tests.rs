use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Source {
    bytes: Vec<u8>,
    length: u64,
    chunk: usize,
    calls: Mutex<Vec<(u64, usize)>>,
    cancel: Option<Cancellation>,
    changing: bool,
    lengths: AtomicUsize,
}
impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            length: bytes.len() as u64,
            chunk: usize::MAX,
            calls: Mutex::default(),
            cancel: None,
            changing: false,
            lengths: AtomicUsize::new(0),
        }
    }
}
impl GeneratedTextSource for Source {
    fn length(&self) -> u64 {
        let previous = self.lengths.fetch_add(1, Ordering::Relaxed);
        self.length + u64::from(self.changing && previous != 0)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.calls.lock().unwrap().push((offset, output.len()));
        let bytes = self.bytes.get(offset as usize..).unwrap_or(&[]);
        let count = output.len().min(bytes.len()).min(self.chunk);
        output[..count].copy_from_slice(&bytes[..count]);
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        }
        Ok(count)
    }
}

#[test]
fn empty_and_short_chunk_unicode_reports_preserve_exact_text() {
    for text in ["", "資料 🦀\r\nPermission denied\n"] {
        let mut source = Source::new(text.as_bytes());
        source.chunk = 2;
        let result = read(&source, &Cancellation::default()).unwrap();
        assert_eq!(result.text, text);
        assert_eq!(result.total_bytes, text.len() as u64);
        assert!(!result.truncated);
    }
}

#[test]
fn huge_reports_read_only_the_preview_budget_in_small_chunks() {
    let mut source = Source::new(&vec![b'x'; DIAGNOSTIC_PREVIEW_BYTES]);
    source.length = u64::MAX;
    let result = read(&source, &Cancellation::default()).unwrap();
    assert_eq!(result.text.len(), DIAGNOSTIC_PREVIEW_BYTES);
    assert_eq!(result.total_bytes, u64::MAX);
    assert!(result.truncated);
    let calls = source.calls.lock().unwrap();
    assert!(calls.iter().all(|(_, length)| *length <= 8 * 1024));
    assert_eq!(
        calls.iter().map(|(_, length)| *length).sum::<usize>(),
        DIAGNOSTIC_PREVIEW_BYTES
    );
}

#[test]
fn byte_budget_drops_only_an_incomplete_final_unicode_scalar() {
    let prefix = "x".repeat(DIAGNOSTIC_PREVIEW_BYTES - 1);
    let text = format!("{prefix}🦀 tail");
    let source = Source::new(text.as_bytes());
    let result = read(&source, &Cancellation::default()).unwrap();
    assert_eq!(result.text, prefix);
    assert_eq!(result.total_bytes, text.len() as u64);
    assert!(result.truncated);
}

#[test]
fn malformed_or_short_reports_do_not_publish_a_misleading_preview() {
    for bytes in [&[0xff][..], &[0xe2, 0x82][..]] {
        assert_eq!(
            read(&Source::new(bytes), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidData
        );
    }
    let mut short = Source::new(b"short");
    short.length = 10;
    assert_eq!(
        read(&short, &Cancellation::default()).unwrap_err().kind,
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn cancellation_before_or_during_read_never_becomes_a_preview() {
    let cancel = Cancellation::default();
    cancel.cancel();
    let source = Source::new(b"report");
    assert_eq!(
        read(&source, &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(source.lengths.load(Ordering::Relaxed), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    let cancel = Cancellation::default();
    let mut source = Source::new(&vec![b'x'; DIAGNOSTIC_PREVIEW_BYTES]);
    source.cancel = Some(cancel.clone());
    assert_eq!(
        read(&source, &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[test]
fn changed_capture_length_is_rejected() {
    let mut source = Source::new(b"report");
    source.changing = true;
    assert_eq!(
        read(&source, &Cancellation::default()).unwrap_err().kind,
        io::ErrorKind::InvalidData
    );
}
