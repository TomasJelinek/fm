//! Byte comparison through a whole session: binary pairs, refused text and the Hex view.
use super::*;
use crate::IdentityVerdict;
use midnight_work_budget::resident::Owner;

fn session_for(
    root: &Path,
    left: &'static [u8],
    right: &'static [u8],
) -> (tokio::runtime::Runtime, Owner, Session, Arc<AtomicUsize>) {
    let paths = paths(root);
    let expected = paths.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == expected[0] {
            memory(left, &counted)
        } else {
            memory(right, &counted)
        }
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_image_viewer_discovery(|| Ok(Vec::new()))
        .with_font_scan(|_| Some(Arc::from([])));
    let session = open(&service, launch(&paths)).unwrap();
    (runtime, owner, session, reads)
}

/// The next frame that satisfies `wanted`.
fn frame_where(session: &Session, wanted: impl Fn(&Frame) -> bool) -> Frame {
    wait_for(|last_status| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) => {
            *last_status = Some(frame.status.clone());
            wanted(&frame).then_some(frame)
        }
        Some(Output::Status(status)) => {
            *last_status = Some(status);
            None
        }
        Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        None => None,
    })
}

#[test]
fn a_binary_pair_is_compared_as_bytes_with_a_summary_and_a_byte_verdict() {
    let root = fixture();
    let (_runtime, _owner, session, _) = session_for(
        root.path(),
        b"\x00\x01\x02\x03\x04\x05\x06\x07",
        b"\x00\x01\x02\xff\x04\x05\x06\x07",
    );
    let first = frame_where(&session, |_| true);
    assert!(first.bytes && first.bytes_only && !first.image);
    assert!(
        first.accessible_text.starts_with("0000  00 01 02 03"),
        "{}",
        first.accessible_text
    );
    let done = frame_where(&session, |frame| frame.identity != IdentityVerdict::Pending);
    assert_eq!(done.identity, IdentityVerdict::Different);
    for part in [
        "File A: not UTF-8 or UTF-16 text",
        "1 differing byte in 1 region",
        "first difference at 0x0003 (3)",
        "8 bytes each",
        "Inputs differ",
    ] {
        assert!(done.status.contains(part), "{part}: {}", done.status);
    }
    session.close();
}

#[test]
fn identical_binary_files_get_their_verdict_from_one_byte_scan() {
    let root = fixture();
    let (_runtime, _owner, session, reads) = session_for(
        root.path(),
        b"\x00\x10\x20\x30\x40",
        b"\x00\x10\x20\x30\x40",
    );
    let done = frame_where(&session, |frame| frame.identity != IdentityVerdict::Pending);
    assert_eq!(done.identity, IdentityVerdict::IdenticalFiles);
    assert!(done.status.contains("Bytes identical"), "{}", done.status);
    // Per file: the text check of the first block, the painted rows and one scan turn.
    assert!(
        reads.load(Ordering::Acquire) <= 6,
        "no second whole-file pass"
    );
    session.close();
}

#[test]
fn a_text_pair_shows_its_bytes_on_request_navigates_them_and_returns_to_text() {
    let root = fixture();
    let leak = |text: String| -> &'static [u8] { Box::leak(text.into_bytes().into_boxed_slice()) };
    let same = "same\n".repeat(20);
    let (_runtime, _owner, session, _) = session_for(
        root.path(),
        leak(format!("{same}line\n")),
        leak(format!("{same}LINE\n")),
    );
    let text = frame_where(&session, |_| true);
    assert!(!text.bytes && !text.bytes_only);
    session.send(Input::Bytes(true)).unwrap();
    let hex = frame_where(&session, |frame| {
        frame.bytes && frame.status.contains("4 differing bytes in 1 region")
    });
    assert!(!hex.bytes_only, "a text pair can return to its text");
    assert!(hex.accessible_text.starts_with("0000  73 61 6D 65"));
    // The region starts at offset 100; the first row then holds "line" and "LINE".
    session
        .send(Input::Difference { backwards: false })
        .unwrap();
    let moved = frame_where(&session, |frame| {
        frame.accessible_text.lines().next().is_some_and(|row| {
            row.split_once('\t').is_some_and(|(left, right)| {
                left.contains("6C 69 6E 65") && right.contains("4C 49 4E 45")
            })
        })
    });
    assert!(moved.bytes && moved.status.contains("first difference at 0x0064 (100)"));
    session.send(Input::Bytes(false)).unwrap();
    let back = frame_where(&session, |frame| !frame.bytes);
    assert!(back.accessible_text.contains("same"));
    session.close();
}

/// Reads that take a while, so a byte scan paints its progress before it ends.
#[derive(Debug)]
struct Slow(Vec<u8>);

impl ContentSource for Slow {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }
    fn display_name(&self) -> String {
        "slow.log".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        std::thread::sleep(Duration::from_millis(40));
        let start = (offset as usize).min(self.0.len());
        let end = (start + length as usize).min(self.0.len());
        Ok(self.0[start..end].to_vec())
    }
}

/// The text's verdict is known before the Hex view starts; taking the scan's progress
/// frames must still let the worker finish the scan without any input.
#[test]
fn the_hex_view_of_a_text_pair_finishes_its_scan_after_progress_frames() {
    let root = fixture();
    let paths = paths(root.path());
    let left = "a line of the log\n".repeat(60_000).into_bytes();
    let mut right = left.clone();
    right.extend_from_slice(b"one more line\n");
    let expected = paths.clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        let bytes = if path == expected[0] {
            left.clone()
        } else {
            right.clone()
        };
        CapturedFile::from_source(Arc::new(Slow(bytes))).map_err(|error| error.to_string())
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_image_viewer_discovery(|| Ok(Vec::new()))
        .with_font_scan(|_| Some(Arc::from([])));
    let session = open(&service, launch(&paths)).unwrap();
    let text = frame_where(&session, |frame| frame.identity != IdentityVerdict::Pending);
    assert_eq!(text.identity, IdentityVerdict::Different);
    session.send(Input::Bytes(true)).unwrap();
    let progress = frame_where(&session, |frame| frame.bytes && frame.scanning);
    assert!(
        progress.status.starts_with("Comparing bytes"),
        "{}",
        progress.status
    );
    let done = frame_where(&session, |frame| frame.bytes && !frame.scanning);
    assert!(
        done.status.starts_with("14 differing bytes in 1 region"),
        "{}",
        done.status
    );
    assert_eq!(
        done.identity,
        IdentityVerdict::Different,
        "the text's verdict stays"
    );
    session.close();
}
