//! Raising the open-file limit at start, and giving the user's programs the original back.
use super::*;

fn limit(soft: Option<u64>, hard: Option<u64>) -> Rlimit {
    Rlimit {
        current: soft,
        maximum: hard,
    }
}

#[test]
fn the_soft_limit_rises_to_the_hard_limit_within_the_cap_and_never_falls() {
    // A desktop session's default: soft 1024 under systemd's hard 524,288.
    assert_eq!(
        raised(limit(Some(1024), Some(524_288))),
        Some(MAX_OPEN_FILES)
    );
    // A lower hard limit is the most there is.
    assert_eq!(raised(limit(Some(1024), Some(4096))), Some(4096));
    // An unlimited hard limit still stops at the cap.
    assert_eq!(raised(limit(Some(1024), None)), Some(MAX_OPEN_FILES));
    // Nothing to raise: at the hard limit, at or above the cap, or unlimited.
    assert_eq!(raised(limit(Some(1024), Some(1024))), None);
    assert_eq!(raised(limit(Some(MAX_OPEN_FILES), Some(1 << 20))), None);
    assert_eq!(raised(limit(Some(1 << 20), Some(1 << 20))), None);
    assert_eq!(raised(limit(None, None)), None);
}

#[test]
fn raising_keeps_within_the_hard_limit_and_is_idempotent() {
    let before = getrlimit(Resource::Nofile);
    let soft = raise_open_file_limit().unwrap();
    let after = getrlimit(Resource::Nofile);
    assert_eq!(after.current, soft);
    assert_eq!(after.maximum, before.maximum, "the hard limit stays");
    if let (Some(before), Some(after)) = (before.current, after.current) {
        assert!(after >= before, "never lowered");
        assert!(
            after <= MAX_OPEN_FILES.max(before),
            "{after} beyond the cap"
        );
    }
    assert_eq!(
        raise_open_file_limit().unwrap(),
        soft,
        "a second call changes nothing"
    );
}

#[test]
fn a_program_started_for_the_user_gets_the_original_limit_back() {
    let hard = getrlimit(Resource::Nofile).maximum;
    // The shell's own view of its limit; 256 stands for the limit the application
    // started with, and is never above a hard limit.
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("ulimit -n");
    restore_in_child(&mut command, limit(Some(256), hard));
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "256");
}
