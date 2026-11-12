use super::*;
use midnight_core::FolderSize;
use std::{
    sync::{Condvar, mpsc},
    thread::{self, ThreadId},
    time::Duration,
};

type Gate = Arc<(Mutex<bool>, Condvar)>;
struct Release(Gate);
impl Release {
    fn open(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.open();
    }
}
struct Completion(mpsc::Sender<()>);
impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
struct ObservedStorage {
    spool: AppendSpool,
    _completed: Completion,
    caller: ThreadId,
    entered: mpsc::Sender<ThreadId>,
    gate: Gate,
}
impl Storage for ObservedStorage {
    fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<u64> {
        self.spool.append(offset, bytes)
    }
    fn length(&self) -> u64 {
        self.spool.length()
    }
    fn prefix(&self, start: u64, end: u64) -> io::Result<SpoolPrefix> {
        self.spool.prefix(start, end)
    }
}
impl Drop for ObservedStorage {
    fn drop(&mut self) {
        let current = thread::current().id();
        let _ = self.entered.send(current);
        // Baseline caller destruction must fail rather than deadlock this test.
        if current != self.caller {
            let _released = self
                .gate
                .1
                .wait_while(self.gate.0.lock().unwrap(), |released| !*released)
                .unwrap();
        }
        // The actual AppendSpool is still alive throughout the gate above.
    }
}
struct Observer {
    dropped: mpsc::Receiver<ThreadId>,
    completed: mpsc::Receiver<()>,
    release: Release,
}
impl Observer {
    fn finish(&self) {
        self.release.open();
        self.completed.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
fn observed(directory: PathBuf) -> (impl FnOnce() -> ReportLog + Send, Observer) {
    observed_with_arena(directory, ReportArena::testing())
}
fn observed_with_arena(
    directory: PathBuf,
    arena: ReportArena,
) -> (impl FnOnce() -> ReportLog + Send, Observer) {
    let caller = thread::current().id();
    let (entered, dropped) = mpsc::channel();
    let (complete, completed) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    (
        move || {
            let reservation = arena.reserve().unwrap();
            let spool = PrivateSpoolDirectory::open(&directory)
                .unwrap()
                .create_append_only()
                .unwrap();
            let log = ReportLog::new(&arena, directory);
            log.0.lock().unwrap().storage =
                Some(reservation.retain(Mutex::new(Box::new(ObservedStorage {
                    spool,
                    _completed: Completion(complete),
                    caller,
                    entered,
                    gate: worker_gate,
                }) as Box<dyn Storage>)));
            log
        },
        Observer {
            dropped,
            completed,
            release: Release(gate),
        },
    )
}
fn contents(source: &dyn GeneratedTextSource) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0_u8; 997];
        let count = source.read_at(bytes.len() as u64, &mut chunk).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    assert_eq!(bytes.len() as u64, source.length());
    String::from_utf8(bytes).unwrap()
}
fn complete_size(make_log: impl FnOnce() -> ReportLog) -> FolderSize {
    let log = make_log();
    let range = log.mark().report();
    let reason = format!("{} final Unicode tail", "資料 failure ".repeat(20_000));
    log.record(
        "東京 folder",
        "read directory",
        io::ErrorKind::PermissionDenied,
        &reason,
    );
    range.finish(false);
    let warnings = range.handle().unwrap();
    let text = warnings.snapshot(&Cancellation::default()).unwrap();
    assert!(contents(text.as_ref()).contains(&reason));
    FolderSize {
        warnings: Some(warnings),
        errors: 1,
        finished: true,
        ..Default::default()
    }
}
#[test]
fn report_retirement_folder_size_final_owner_destroys_actual_storage_off_caller() {
    let temporary = tempfile::tempdir().unwrap();
    let lifetime = crate::Engine::lifetime().unwrap();
    let (make_log, observer) =
        observed_with_arena(temporary.path().join("private"), lifetime.report_arena());
    let size = thread::spawn(move || complete_size(make_log))
        .join()
        .unwrap();
    // Real public startup context can disappear while an independent result survives.
    drop(lifetime);
    let retained = size.clone();
    drop(size);
    assert!(
        observer.dropped.try_recv().is_err(),
        "a retained output must keep its file"
    );
    drop(retained);
    let actual = observer
        .dropped
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_ne!(
        actual,
        thread::current().id(),
        "actual AppendSpool owner dropped on caller"
    );
    observer.finish();
}
#[test]
fn report_retirement_completed_unconsumed_output_survives_runtime_shutdown() {
    let temporary = tempfile::tempdir().unwrap();
    let (make_log, observer) = observed(temporary.path().join("private"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let output = runtime.spawn_blocking(move || complete_size(make_log));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !output.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "native report preparation timed out"
        );
        thread::yield_now();
    }
    drop(runtime);
    assert!(
        observer.dropped.try_recv().is_err(),
        "completed output still owns the report"
    );
    drop(output);
    assert_ne!(
        observer
            .dropped
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        thread::current().id()
    );
    observer.finish();
}
#[test]
fn report_retirement_unpolled_work_keeps_native_cleanup_off_caller() {
    let temporary = tempfile::tempdir().unwrap();
    let (make_log, observer) = observed(temporary.path().join("private"));
    let size = thread::spawn(move || complete_size(make_log))
        .join()
        .unwrap();
    let unpolled = async move {
        drop(size);
    };
    drop(unpolled);
    assert_ne!(
        observer
            .dropped
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        thread::current().id()
    );
    observer.finish();
}

#[cfg(target_os = "linux")]
#[path = "size_warning_retirement_linux_tests.rs"]
mod linux;
