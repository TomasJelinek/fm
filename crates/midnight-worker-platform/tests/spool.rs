use midnight_worker_platform::spool::{MAX_IO, PrivateSpoolDirectory};
use std::{io, sync::Arc};

#[test]
fn private_spool_requires_seal_and_supports_checked_random_reads() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    assert_eq!(spool.length(), 0);
    assert!(spool.read_at(0, &mut [0; 1]).is_err());
    assert_eq!(spool.append(0, b"abc").unwrap(), 3);
    assert!(spool.append(1, b"x").is_err());
    assert_eq!(spool.append(3, b"def").unwrap(), 6);
    spool.finish().unwrap();
    assert!(spool.append(6, b"x").is_err());
    assert!(spool.finish().is_err());
    let mut bytes = [0; 4];
    assert_eq!(spool.read_at(1, &mut bytes).unwrap(), 4);
    assert_eq!(&bytes, b"bcde");
    assert_eq!(spool.read_at(5, &mut bytes).unwrap(), 1);
    assert_eq!(bytes[0], b'f');
    assert_eq!(spool.read_at(6, &mut bytes).unwrap(), 0);
    assert_eq!(spool.read_at(99, &mut bytes).unwrap(), 0);
    assert!(spool.read_at(u64::MAX, &mut bytes).is_err());
}

#[test]
fn rejected_inputs_do_not_poison_a_valid_spool() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    let oversized = vec![0; MAX_IO + 1];
    assert_eq!(
        spool.append(0, &oversized).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(spool.append(u64::MAX, &[1]).is_err());
    assert!(spool.append(0, &[]).is_err());
    assert_eq!(spool.append(0, b"valid").unwrap(), 5);
    spool.finish().unwrap();
    assert!(spool.read_at(0, &mut vec![0; MAX_IO + 1]).is_err());
    assert_eq!(spool.read_at(0, &mut []).unwrap(), 0);
    let mut bytes = [0; 5];
    spool.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"valid");
}

#[test]
fn independent_spools_retain_root_and_allow_concurrent_readers() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("private");
    let root = PrivateSpoolDirectory::open(&path).unwrap();
    let mut first = root.create().unwrap();
    let mut second = root.create().unwrap();
    first.append(0, b"first").unwrap();
    second.append(0, b"second").unwrap();
    first.finish().unwrap();
    second.finish().unwrap();
    drop(root);
    let first = Arc::new(first);
    let threads: Vec<_> = (0..4)
        .map(|index| {
            let first = first.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    let mut bytes = [0; 2];
                    assert_eq!(first.read_at(index, &mut bytes).unwrap(), 2);
                    assert_eq!(&bytes, &b"first"[index as usize..index as usize + 2]);
                }
            })
        })
        .collect();
    let mut bytes = [0; 6];
    second.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"second");
    for thread in threads {
        thread.join().unwrap();
    }
    drop(first);
    drop(second);
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
}

#[test]
fn empty_spools_and_chunk_boundary_are_valid() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut empty = root.create().unwrap();
    empty.finish().unwrap();
    assert_eq!(empty.read_at(0, &mut [0; 1]).unwrap(), 0);
    let mut spool = root.create().unwrap();
    let bytes = vec![0xa5; MAX_IO];
    spool.append(0, &bytes).unwrap();
    spool.append(MAX_IO as u64, &bytes).unwrap();
    spool.finish().unwrap();
    let mut output = vec![0; MAX_IO];
    assert_eq!(
        spool.read_at(MAX_IO as u64 / 2, &mut output).unwrap(),
        MAX_IO
    );
    assert_eq!(output, bytes);
}

#[test]
fn process_termination_removes_unfinished_spool_content() {
    const CHILD: &str = "MIDNIGHT_SPOOL_CRASH_TEST";
    if let Some(parent) = std::env::var_os(CHILD) {
        let parent = std::path::PathBuf::from(parent);
        let root = PrivateSpoolDirectory::open(&parent.join("private")).unwrap();
        let mut spool = root.create().unwrap();
        spool.append(0, b"unpublished private bytes").unwrap();
        drop(root);
        std::fs::write(parent.join("ready"), b"ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_termination_removes_unfinished_spool_content",
        ])
        .env(CHILD, temporary.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !temporary.path().join("ready").exists() {
        if child.try_wait().unwrap().is_some() || std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("spool child did not become ready");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        std::fs::read_dir(temporary.path().join("private"))
            .unwrap()
            .count(),
        0
    );
}
