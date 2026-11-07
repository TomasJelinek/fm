//! Linux permits existing writers and namespace substitution while files are held.
use super::*;

fn wait_before_capture(job: &mut worker::Job, status: &Status) {
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(status).phase == CopyOperationPhase::ValidatingBeforeMove {
            return;
        }
    }
    panic!("Move did not reach its complete pre-capture proof pass");
}

#[test]
fn move_source_content_or_name_change_before_barrier_keeps_every_source() {
    for change_name in [false, true] {
        let (_temp, parent) = protected_fixture();
        let paths = [parent.join("first"), parent.join("second")];
        let destination = parent.join("destination");
        fs::create_dir(&destination).unwrap();
        for path in &paths {
            fs::write(path, b"original").unwrap();
        }
        let (mut job, status) = prepare_move(&paths, &destination, Cancellation::default());
        wait_before_capture(&mut job, &status);
        if change_name {
            fs::rename(&paths[0], parent.join("retained-source")).unwrap();
        }
        fs::write(&paths[0], b"modified").unwrap();
        drive(&mut job);
        let done = job.finish();
        assert!(done.failed > 0, "{done:?}");
        assert_eq!(done.source_names_captured, 0);
        assert_eq!(fs::read(&paths[0]).unwrap(), b"modified");
        assert_eq!(fs::read(&paths[1]).unwrap(), b"original");
        assert_eq!(fs::read(destination.join("first")).unwrap(), b"original");
        if change_name {
            assert_eq!(
                fs::read(parent.join("retained-source")).unwrap(),
                b"original"
            );
        }
    }
}

#[test]
fn move_destination_substitution_before_barrier_is_preserved_and_blocks_all_capture() {
    let (_temp, parent) = protected_fixture();
    let paths = [parent.join("first"), parent.join("second")];
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, b"original").unwrap();
    }
    let (mut job, status) = prepare_move(&paths, &destination, Cancellation::default());
    wait_before_capture(&mut job, &status);
    fs::rename(
        destination.join("first"),
        destination.join("retained-output"),
    )
    .unwrap();
    fs::write(destination.join("first"), b"new winner").unwrap();
    drive(&mut job);
    let done = job.finish();
    assert!(done.failed > 0);
    assert_eq!(done.source_names_captured, 0);
    assert!(paths.iter().all(|path| path.exists()));
    assert_eq!(fs::read(destination.join("first")).unwrap(), b"new winner");
    assert_eq!(
        fs::read(destination.join("retained-output")).unwrap(),
        b"original"
    );
}

#[test]
fn move_late_writer_bytes_survive_in_recovery_and_final_pass_reports_mismatch() {
    use std::io::{Seek, SeekFrom, Write};
    let (_temp, parent) = protected_fixture();
    let paths = [parent.join("first"), parent.join("second")];
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, b"original").unwrap();
    }
    let mut writer = fs::OpenOptions::new().write(true).open(&paths[0]).unwrap();
    let (mut job, status) = prepare_move(&paths, &destination, Cancellation::default());
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_names_captured == 1 {
            break;
        }
    }
    assert_eq!(snapshot(&status).source_names_captured, 1);
    writer.seek(SeekFrom::Start(0)).unwrap();
    writer.write_all(b"modified").unwrap();
    writer.sync_all().unwrap();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::CompletedWithErrors);
    assert!(done.failed > 0);
    let original = done
        .source_recoveries
        .iter()
        .find(|item| item.source == paths[0])
        .unwrap();
    assert!(original.matched_source);
    assert_eq!(fs::read(&original.recovery.path).unwrap(), b"modified");
    assert_eq!(fs::read(destination.join("first")).unwrap(), b"original");
}
