use super::*;
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn recursive_delete_pauses_after_one_entry_then_resumes_or_cancels_without_more_deletions() {
    for cancel_instead in [false, true] {
        let qa = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/pause-delete");
        fs::create_dir_all(&qa).unwrap();
        let fixture = tempfile::tempdir_in(qa.canonicalize().unwrap()).unwrap();
        let tree = fixture.path().join("tree");
        fs::create_dir(&tree).unwrap();
        for name in ["a", "b", "c"] {
            fs::write(tree.join(name), name).unwrap();
        }
        let cancel = Cancellation::default().pausable_child();
        let prepared = WindowsFileSystem::new(None)
            .prepare(&tree, &cancel)
            .unwrap();
        let worker_cancel = cancel.clone();
        let completed = Arc::new(AtomicUsize::new(0));
        let observed = completed.clone();
        let (sent, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = prepared.delete(DeleteMode::Permanent, &worker_cancel, &mut |_| {
                if observed.fetch_add(1, Ordering::AcqRel) == 0 {
                    worker_cancel.set_pause_requested(true);
                }
            });
            sent.send(result).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cancel.is_paused() {
            assert!(
                Instant::now() < deadline,
                "delete failed to reach paused checkpoint"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(completed.load(Ordering::Acquire), 1);
        assert!(received.recv_timeout(Duration::from_millis(40)).is_err());
        assert_eq!(completed.load(Ordering::Acquire), 1);
        if cancel_instead {
            cancel.cancel();
        } else {
            cancel.set_pause_requested(false);
        }
        let result = received.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
        if cancel_instead {
            assert_eq!(result.unwrap_err().kind, io::ErrorKind::Interrupted);
            assert_eq!(completed.load(Ordering::Acquire), 1);
            assert_eq!(fs::read_dir(&tree).unwrap().count(), 2);
            assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
        } else {
            result.unwrap();
            assert_eq!(completed.load(Ordering::Acquire), 4);
            assert!(!tree.exists());
        }
    }
}
