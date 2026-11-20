use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A file whose content and native revision a test replaces after capture, as a rewrite or
/// a metadata-only update (an alternate data stream added by endpoint protection) would.
#[derive(Debug)]
struct File {
    /// Content and revision.
    state: Mutex<(Vec<u8>, Vec<u8>)>,
    reads: AtomicUsize,
}

impl File {
    fn new(content: &[u8], revision: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((content.to_vec(), revision)),
            reads: AtomicUsize::new(0),
        })
    }

    fn set(&self, content: &[u8], revision: Vec<u8>) {
        *self.state.lock().unwrap() = (content.to_vec(), revision);
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::Acquire)
    }

    fn capture(self: &Arc<Self>) -> Result<CapturedFile, String> {
        CapturedFile::from_source(self.clone()).map_err(|error| error.to_string())
    }
}

impl ContentSource for File {
    fn length(&self) -> u64 {
        self.state.lock().unwrap().0.len() as u64
    }
    fn display_name(&self) -> String {
        "drift.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        self.state.lock().unwrap().1.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        let content = &self.state.lock().unwrap().0;
        let start = (offset as usize).min(content.len());
        let end = start.saturating_add(length as usize).min(content.len());
        Ok(content[start..end].to_vec())
    }
}

/// A Windows revision as the native filesystem plugin writes it: volume, file ID, length,
/// LastWriteTime and ChangeTime.
fn windows(volume: u64, id: u8, length: u64, write: i64, change: i64) -> Vec<u8> {
    windows_with(b"native.windows.read.v1", volume, id, length, write, change)
}

fn windows_with(
    prefix: &[u8],
    volume: u64,
    id: u8,
    length: u64,
    write: i64,
    change: i64,
) -> Vec<u8> {
    [
        prefix,
        &volume.to_le_bytes(),
        &[id; 16],
        &length.to_le_bytes(),
        &write.to_le_bytes(),
        &change.to_le_bytes(),
    ]
    .concat()
}

/// A Linux revision: device, inode, length, mtime and ctime (seconds, nanoseconds).
fn linux(prefix: &[u8], device: u64, inode: u64, length: u64, times: [i64; 4]) -> Vec<u8> {
    let mut bytes = [
        prefix,
        &device.to_le_bytes(),
        &inode.to_le_bytes(),
        &length.to_le_bytes(),
    ]
    .concat();
    for time in times {
        bytes.extend(time.to_le_bytes());
    }
    bytes
}

#[test]
fn only_a_moved_change_time_qualifies_in_each_native_layout() {
    for prefix in [b"native.windows.read.v1".as_slice(), b"windows-file-v1"] {
        let captured = windows_with(prefix, 7, 1, 10, 100, 200);
        assert!(change_time_only(
            &captured,
            &windows_with(prefix, 7, 1, 10, 100, 201)
        ));
        for current in [
            windows_with(prefix, 7, 1, 10, 100, 200),
            windows_with(prefix, 8, 1, 10, 100, 201),
            windows_with(prefix, 7, 2, 10, 100, 201),
            windows_with(prefix, 7, 1, 11, 100, 201),
            windows_with(prefix, 7, 1, 10, 101, 201),
            windows_with(prefix, 7, 1, 10, 101, 200),
        ] {
            assert!(!change_time_only(&captured, &current), "{current:?}");
        }
    }
    for prefix in [b"native.linux.read.v1".as_slice(), b"linux-file-v1"] {
        let captured = linux(prefix, 3, 4, 10, [100, 5, 200, 6]);
        for current in [
            linux(prefix, 3, 4, 10, [100, 5, 201, 6]),
            linux(prefix, 3, 4, 10, [100, 5, 200, 7]),
        ] {
            assert!(change_time_only(&captured, &current), "{current:?}");
        }
        for current in [
            linux(prefix, 3, 4, 10, [100, 5, 200, 6]),
            linux(prefix, 9, 4, 10, [100, 5, 201, 6]),
            linux(prefix, 3, 9, 10, [100, 5, 201, 6]),
            linux(prefix, 3, 4, 11, [100, 5, 201, 6]),
            linux(prefix, 3, 4, 10, [101, 5, 201, 6]),
            linux(prefix, 3, 4, 10, [100, 9, 201, 6]),
        ] {
            assert!(!change_time_only(&captured, &current), "{current:?}");
        }
    }
    // An unknown format, another capture prefix and a malformed payload never qualify.
    assert!(!change_time_only(
        &windows_with(b"native.windows.read.v2", 7, 1, 10, 100, 200),
        &windows_with(b"native.windows.read.v2", 7, 1, 10, 100, 201)
    ));
    let captured = windows(7, 1, 10, 100, 200);
    let mut truncated = windows(7, 1, 10, 100, 201);
    truncated.pop();
    let mut extended = windows(7, 1, 10, 100, 201);
    extended.push(0);
    for current in [
        windows_with(b"windows-file-v1", 7, 1, 10, 100, 201),
        windows_with(b"native.macos.read.v1", 7, 1, 10, 100, 201),
        truncated,
        extended,
    ] {
        assert!(!change_time_only(&captured, &current), "{current:?}");
    }
    assert!(!change_time_only(&[1], &[2]));
}

/// Two captured files and the text the comparison retained from them.
struct Comparison {
    files: [Arc<File>; 2],
    sources: [CapturedFile; 2],
    text: TextComparison,
    recaptures: AtomicUsize,
}

/// The text a comparison retains after its initial reads of `sources`.
fn retained(sources: &[CapturedFile; 2]) -> TextComparison {
    let cancel = AtomicBool::new(false);
    let inputs = sources
        .each_ref()
        .map(|source| text::read(source, false, &cancel).unwrap());
    let mut text = TextComparison::new(&inputs[0].text, &inputs[1].text, &cancel).unwrap();
    text.encodings = Some(inputs.each_ref().map(|input| input.encoding));
    text
}

impl Comparison {
    fn new(left: &[u8], right: &[u8]) -> Self {
        let files = [left, right]
            .map(|content| File::new(content, windows(7, 1, content.len() as u64, 100, 200)));
        let sources = files.each_ref().map(|file| file.capture().unwrap());
        Self {
            text: retained(&sources),
            files,
            sources,
            recaptures: AtomicUsize::new(0),
        }
    }

    fn revalidate(&mut self, retained: bool, cancel: bool) -> Result<bool, String> {
        let files = &self.files;
        let recaptures = &self.recaptures;
        revalidate(
            &mut self.sources,
            retained.then_some(&self.text),
            |side| {
                recaptures.fetch_add(1, Ordering::AcqRel);
                files[side].capture()
            },
            [false; 2],
            &AtomicBool::new(cancel),
        )
    }

    fn reads(&self) -> [usize; 2] {
        self.files.each_ref().map(|file| file.reads())
    }

    fn versions(&self) -> [Vec<u8>; 2] {
        self.sources.each_ref().map(CapturedFile::stable_version)
    }
}

#[test]
fn a_change_time_drift_with_identical_content_adopts_the_fresh_capture() {
    let mut comparison = Comparison::new(b"same\nleft\n", b"same\nright\n");
    let left = comparison.sources[0].stable_version();
    let drifted = windows(7, 1, 11, 100, 201);
    comparison.files[1].set(b"same\nright\n", drifted.clone());
    let before = comparison.reads();
    assert_eq!(comparison.revalidate(true, false), Ok(true));
    assert_eq!(comparison.versions(), [left, drifted]);
    let after = comparison.reads();
    assert_eq!(after[0], before[0]);
    assert!(after[1] > before[1], "the fresh capture's content was read");
    // The adopted revision validates: a later check captures and reads nothing.
    assert_eq!(comparison.revalidate(true, false), Ok(false));
    assert_eq!(comparison.reads(), after);
    assert_eq!(comparison.recaptures.load(Ordering::Acquire), 1);
}

/// A real permission change moves only the change time: the read-only attribute and NTFS
/// ChangeTime on Windows, the mode and ctime on Linux. Captured in-process, as
/// `Service::new` does, the drifted file is adopted after its content is read again.
#[test]
fn a_real_permission_change_is_adopted_through_the_in_process_capture() {
    let directory = tempfile::tempdir().unwrap();
    // No extension: endpoint protection on some machines rescans fresh `.txt` files.
    let paths = ["left", "right"].map(|name| directory.path().join(name));
    for (path, content) in paths.iter().zip(["same\nleft\n", "same\nright\n"]) {
        std::fs::write(path, content).unwrap();
    }
    let capture =
        |side: usize| CapturedFile::capture(&paths[side]).map_err(|error| error.to_string());
    let mut sources = [0, 1].map(|side| capture(side).unwrap());
    let text = retained(&sources);
    let captured = sources[0].stable_version();
    // Leave the clock past the write's timestamp, which may be as coarse as a tick.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let writable = std::fs::metadata(&paths[0]).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&paths[0], read_only).unwrap();
    let drifted = sources[0].revalidate().is_err();
    let result = revalidate(
        &mut sources,
        Some(&text),
        capture,
        [false; 2],
        &AtomicBool::new(false),
    );
    std::fs::set_permissions(&paths[0], writable).unwrap();
    assert!(drifted, "the permission change moved the captured revision");
    assert_eq!(result, Ok(true));
    assert!(change_time_only(&captured, &sources[0].stable_version()));
}

#[test]
fn a_change_time_drift_with_different_or_unreadable_content_is_a_change() {
    // Each rewrite keeps the length and LastWriteTime, so only the content reveals it.
    for content in [
        b"same\nLeft\n".as_slice(),
        b"same\nle\x01t\n",
        b"same\nle\xfft\n",
    ] {
        let mut comparison = Comparison::new(b"same\nleft\n", b"same\nright\n");
        let captured = comparison.versions();
        comparison.files[0].set(content, windows(7, 1, 10, 100, 201));
        let error = comparison.revalidate(true, false).unwrap_err();
        assert!(error.starts_with("Compared file changed: "), "{error}");
        assert_eq!(comparison.versions(), captured);
        assert_eq!(comparison.recaptures.load(Ordering::Acquire), 1);
    }
}

#[test]
fn an_identity_length_or_write_time_change_is_a_change_without_a_content_read() {
    for (content, revision) in [
        (b"same\nleft\n".as_slice(), windows(8, 1, 10, 100, 201)),
        (b"same\nleft\n", windows(7, 2, 10, 100, 201)),
        (b"same\nleft!\n", windows(7, 1, 11, 100, 201)),
        (b"same\nleft\n", windows(7, 1, 10, 101, 201)),
        (b"same\nleft\n", windows(7, 1, 10, 101, 200)),
    ] {
        let mut comparison = Comparison::new(b"same\nleft\n", b"same\nright\n");
        let captured = comparison.versions();
        comparison.files[0].set(content, revision);
        let before = comparison.reads();
        let error = comparison.revalidate(true, false).unwrap_err();
        assert!(error.starts_with("Compared file changed: "), "{error}");
        assert_eq!(comparison.reads(), before, "no content was read");
        assert_eq!(comparison.versions(), captured);
    }
}

#[test]
fn image_sources_report_a_change_time_drift_without_a_capture_or_read() {
    let mut comparison = Comparison::new(b"same\nleft\n", b"same\nright\n");
    comparison.files[0].set(b"same\nleft\n", windows(7, 1, 10, 100, 201));
    let before = comparison.reads();
    let error = comparison.revalidate(false, false).unwrap_err();
    assert!(error.starts_with("Compared file changed: "), "{error}");
    assert_eq!(comparison.reads(), before);
    assert_eq!(comparison.recaptures.load(Ordering::Acquire), 0);
}

#[test]
fn a_cancelled_content_check_reports_the_cancellation_and_keeps_the_source() {
    let mut comparison = Comparison::new(b"same\nleft\n", b"same\nright\n");
    let captured = comparison.versions();
    comparison.files[0].set(b"same\nleft\n", windows(7, 1, 10, 100, 201));
    let before = comparison.reads();
    assert_eq!(
        comparison.revalidate(true, true),
        Err("Comparison cancelled.".into())
    );
    assert_eq!(comparison.reads(), before);
    assert_eq!(comparison.versions(), captured);
}

#[test]
fn an_initial_read_interrupted_by_a_change_time_drift_reads_the_fresh_capture() {
    let file = File::new(b"same\nleft\n", windows(7, 1, 10, 100, 200));
    let mut source = file.capture().unwrap();
    let drifted = windows(7, 1, 10, 100, 201);
    file.set(b"same\nleft\n", drifted.clone());
    assert_eq!(
        initial_text(&mut source, || file.capture()),
        Ok("same\nleft\n".into())
    );
    assert_eq!(source.stable_version(), drifted);
}

fn initial_text(
    source: &mut CapturedFile,
    recapture: impl FnOnce() -> Result<CapturedFile, String>,
) -> Result<String, String> {
    read_text(source, recapture, false, &AtomicBool::new(false)).map(|input| input.text)
}

#[test]
fn an_initial_read_keeps_every_other_failure() {
    let file = File::new(b"same\nleft\n", windows(7, 1, 10, 100, 200));
    let mut source = file.capture().unwrap();
    let captured = source.stable_version();
    file.set(b"same\nleft\n", windows(7, 1, 10, 101, 201));
    let reads = file.reads();
    let error = initial_text(&mut source, || file.capture()).unwrap_err();
    assert!(error.contains("Source changed since capture"), "{error}");
    assert_eq!(file.reads(), reads);
    assert_eq!(source.stable_version(), captured);
    // A source that still validates failed for its content: no capture is taken.
    let binary = File::new(b"\x01\n", windows(7, 1, 2, 100, 200));
    let mut source = binary.capture().unwrap();
    let error = initial_text(&mut source, || {
        unreachable!("a valid source is not captured again")
    })
    .unwrap_err();
    assert!(error.contains("binary control characters"), "{error}");
}
