use super::*;
use midnight_core::GeneratedTextSource;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct Content {
    bytes: Vec<u8>,
    reported: u64,
    reads: AtomicUsize,
}
impl GeneratedTextSource for Content {
    fn length(&self) -> u64 {
        self.reported
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let offset = offset as usize;
        let count = output
            .len()
            .min(2)
            .min(self.bytes.len().saturating_sub(offset));
        output[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        Ok(count)
    }
}
fn source(bytes: &[u8], reported: u64) -> Arc<Content> {
    Arc::new(Content {
        bytes: bytes.to_vec(),
        reported,
        reads: AtomicUsize::new(0),
    })
}

#[test]
fn generated_clipboard_reads_unicode_short_chunks_exactly() {
    let text = "資料 🦀\r\n";
    let input = source(text.as_bytes(), text.len() as u64);
    let Prepared::Text(actual) = prepare(
        &Action::Generated(input.clone(), Cancellation::default()),
        &Cancellation::default(),
    )
    .unwrap() else {
        panic!("expected text")
    };
    assert_eq!(actual, text);
    assert!(input.reads.load(Ordering::Relaxed) > 1);
}

#[test]
fn generated_clipboard_rejects_truncation_invalid_utf8_nul_and_oversize() {
    for input in [
        source(b"abc", 5),
        source(&[255], 1),
        source(b"a\0b", 3),
        source(b"", 4 * 1024 * 1024 + 1),
    ] {
        assert!(
            prepare(
                &Action::Generated(input, Cancellation::default()),
                &Cancellation::default()
            )
            .is_err()
        );
    }
    let large = source(b"", 4 * 1024 * 1024 + 1);
    assert!(
        prepare(
            &Action::Generated(large.clone(), Cancellation::default()),
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(large.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn cancelled_empty_generated_clipboard_never_becomes_a_write() {
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        matches!(prepare(&Action::Generated(source(b"",0), Cancellation::default()), &cancel), Err(e) if e.kind == io::ErrorKind::Interrupted)
    );
}

#[test]
fn cancelled_request_lease_rejects_even_empty_generated_output() {
    let lease = Cancellation::default();
    lease.cancel();
    for content in [source(b"", 0), source(b"text", 4)] {
        assert!(
            matches!(prepare(&Action::Generated(content.clone(), lease.clone()), &Cancellation::default()), Err(error) if error.kind == io::ErrorKind::Interrupted)
        );
        assert_eq!(content.reads.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn queued_generated_copy_keeps_cancellable_lease_until_worker_preparation() {
    let (sender, mut queued) = mpsc::channel(1);
    let (events, _received) = mpsc::channel(1);
    let mut controller = Controller::new(
        None,
        Outbox {
            sender: events,
            wakeup: Arc::new(|| {}),
        },
    );
    controller.sender = Some(sender);
    let mut tasks = JoinSet::new();
    let lease = Cancellation::default();
    controller
        .copy_generated(
            DesktopRequestId(17),
            source(b"text", 4),
            lease.clone(),
            &mut tasks,
        )
        .await;
    lease.cancel();
    let request = queued.recv().await.unwrap();
    assert!(
        matches!(prepare(&request.action, &Cancellation::default()), Err(error) if error.kind == io::ErrorKind::Interrupted)
    );
    assert!(tasks.is_empty());
    controller.shutdown();
}

struct CancelOnOpen {
    lease: Cancellation,
    writes: Arc<AtomicUsize>,
}
struct Sink(Arc<AtomicUsize>);
impl ClipboardService for CancelOnOpen {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        self.lease.cancel();
        Ok(Box::new(Sink(self.writes.clone())))
    }
}
impl ClipboardSession for Sink {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn cancellation_during_native_open_is_checked_before_clipboard_mutation() {
    let (sender, mut events) = mpsc::channel(1);
    let lease = Cancellation::default();
    let writes = Arc::new(AtomicUsize::new(0));
    let mut controller = Controller::new(
        Some(Arc::new(CancelOnOpen {
            lease: lease.clone(),
            writes: writes.clone(),
        })),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let mut tasks = JoinSet::new();
    controller
        .copy_generated(DesktopRequestId(18), source(b"text", 4), lease, &mut tasks)
        .await;
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    assert_eq!(writes.load(Ordering::Relaxed), 0);
    assert!(
        matches!(event, Event::DesktopActionFinished { result: Err(error), .. } if error.kind == io::ErrorKind::Interrupted)
    );
}

#[derive(Debug)]
struct ChangingLength(AtomicUsize);
impl GeneratedTextSource for ChangingLength {
    fn length(&self) -> u64 {
        if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
            1
        } else {
            4 * 1024 * 1024 + 1
        }
    }
    fn read_at(&self, _: u64, output: &mut [u8]) -> io::Result<usize> {
        assert_eq!(
            output.len(),
            1,
            "allocation must use the budget-checked length"
        );
        output[0] = b'x';
        Ok(1)
    }
}

#[test]
fn changing_reported_length_cannot_enlarge_the_clipboard_allocation() {
    let action = Action::Generated(
        Arc::new(ChangingLength(AtomicUsize::new(0))),
        Cancellation::default(),
    );
    assert!(
        matches!(prepare(&action, &Cancellation::default()), Err(error) if error.kind == io::ErrorKind::InvalidData)
    );
}

#[derive(Debug)]
struct CancelOnRead(Cancellation);
impl GeneratedTextSource for CancelOnRead {
    fn length(&self) -> u64 {
        1
    }
    fn read_at(&self, _: u64, output: &mut [u8]) -> io::Result<usize> {
        output[0] = b'x';
        self.0.cancel();
        Ok(1)
    }
}

#[test]
fn cancellation_in_final_read_cannot_prepare_a_clipboard_write() {
    let lease = Cancellation::default();
    let action = Action::Generated(Arc::new(CancelOnRead(lease.clone())), lease);
    assert!(
        matches!(prepare(&action, &Cancellation::default()), Err(error) if error.kind == io::ErrorKind::Interrupted)
    );
}
