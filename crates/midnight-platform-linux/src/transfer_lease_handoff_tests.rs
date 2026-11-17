use super::*;
use std::{thread, time::Duration};

struct Worker {
    requests: Option<mpsc::SyncSender<Request>>,
    paused: mpsc::Receiver<bool>,
    resume: Option<mpsc::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}
impl Worker {
    fn new() -> Self {
        let (requests, receive) = mpsc::sync_channel(1);
        let (entered, paused) = mpsc::channel();
        let (resume, released) = mpsc::channel();
        let join = thread::spawn(move || {
            serve(receive, |granted| {
                entered.send(granted).unwrap();
                // Closing the channel also releases the worker on assertion unwind.
                let _ = released.recv();
            })
        });
        Self {
            requests: Some(requests),
            paused,
            resume: Some(resume),
            join: Some(join),
        }
    }
    fn requests(&self) -> &mpsc::SyncSender<Request> {
        self.requests.as_ref().unwrap()
    }
    fn wait_paused(&self) -> bool {
        self.paused.recv_timeout(Duration::from_secs(5)).unwrap()
    }
    fn finish(mut self) {
        drop(self.requests.take());
        self.resume.take().unwrap().send(()).unwrap();
        self.join.take().unwrap().join().unwrap();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.requests.take());
        drop(self.resume.take());
    }
}
fn descriptor_no_longer_names(fd: i32, path: &Path) {
    // The descriptor number can be reused by a parallel test, but cannot still
    // identify this test's unique file after the actual File owner is gone.
    assert_ne!(
        std::fs::read_link(format!("/proc/self/fd/{fd}"))
            .ok()
            .as_deref(),
        Some(path)
    );
}

#[test]
fn successful_reply_moves_the_actual_file_owner_before_worker_can_continue() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"lease handoff").unwrap();
    let file = Arc::new(File::open(&path).unwrap());
    let weak = Arc::downgrade(&file);
    let fd = file.as_raw_fd();
    let worker = Worker::new();
    let lease = ReadLease::acquire_with(file, &path, worker.requests()).unwrap();
    assert!(worker.wait_paused());
    lease.revalidate(&path).unwrap();
    assert_eq!(
        weak.strong_count(),
        1,
        "paused worker retained another file reference after success"
    );
    drop(lease);
    assert!(
        weak.upgrade().is_none(),
        "retirement must not await the worker's next loop iteration"
    );
    descriptor_no_longer_names(fd, &path);
    worker.finish();
}

#[test]
fn rejected_native_grant_drops_worker_file_before_error_acknowledgement() {
    let temp = tempfile::tempdir().unwrap();
    // Linux leases are for regular files; a directory is a deterministic native refusal.
    let file = Arc::new(File::open(temp.path()).unwrap());
    let weak = Arc::downgrade(&file);
    let fd = file.as_raw_fd();
    let worker = Worker::new();
    assert!(ReadLease::acquire_with(file, temp.path(), worker.requests()).is_err());
    assert!(!worker.wait_paused());
    assert!(
        weak.upgrade().is_none(),
        "error acknowledgement preceded worker file destruction"
    );
    descriptor_no_longer_names(fd, temp.path());
    worker.finish();
}

#[test]
fn abandoned_success_reply_unlocks_and_drops_its_returned_file_locally() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"abandoned handoff").unwrap();
    let file = Arc::new(File::open(&path).unwrap());
    let fd = file.as_raw_fd();
    let weak = Arc::downgrade(&file);
    let worker = Worker::new();
    let (response, received) = mpsc::sync_channel(1);
    drop(received);
    assert!(
        worker
            .requests()
            .send(Request {
                file: file.clone(),
                response
            })
            .is_ok()
    );
    assert!(
        worker.wait_paused(),
        "the abandoned success test requires an actual granted native lease"
    );
    assert_eq!(Arc::strong_count(&file), 1);
    // SAFETY: the externally retained test file keeps this descriptor valid.
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETLEASE) }, libc::F_UNLCK);
    drop(file);
    assert!(weak.upgrade().is_none());
    descriptor_no_longer_names(fd, &path);
    worker.finish();
}
