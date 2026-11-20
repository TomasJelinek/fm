use super::*;

fn failed(message: &str) -> FsError {
    FsError::from_io("publish", Path::new("/data"), io::Error::other(message))
}

fn kept(path: &str) -> TransferRecovery {
    TransferRecovery {
        path: path.into(),
        identity: None,
    }
}

#[test]
fn a_replacement_reports_the_kept_original_and_every_published_outcome_says_so() {
    let replaced = outcome(
        TransferPublication::Replaced {
            path: "/data/video.mkv".into(),
            recovery: kept("/data/.midnight-recovery-1/item"),
        },
        0,
        true,
    )
    .unwrap();
    assert_eq!(replaced.path, Path::new("/data/video.mkv"));
    assert_eq!(
        replaced.recovery,
        [PathBuf::from("/data/.midnight-recovery-1/item")]
    );
    assert!(replaced.warning.is_none());

    // Published, but the folder sync failed: the file is in place, so this is a
    // warning that says what was done, never a plain failure.
    let unconfirmed = outcome(
        TransferPublication::PublishedDurabilityUnconfirmed {
            path: "/data/文档.txt".into(),
            error: failed("sync failed"),
        },
        0,
        true,
    )
    .unwrap();
    let warning = unconfirmed.warning.expect("durability stays reported");
    assert!(
        warning
            .message
            .starts_with("/data/文档.txt was replaced, but"),
        "{}",
        warning.message
    );
    assert!(warning.message.contains("sync failed"));

    // The exchange happened and the original is retained, but its proof failed.
    let retained = outcome(
        TransferPublication::RecoveryRequired {
            published: Some("/data/a.sha256".into()),
            recovery: vec![kept("/data/.midnight-recovery-2/item")],
            error: failed("a concurrent destination change was retained in recovery"),
        },
        0,
        true,
    )
    .unwrap();
    assert_eq!(
        retained.recovery,
        [PathBuf::from("/data/.midnight-recovery-2/item")]
    );
    assert!(
        retained
            .warning
            .is_some_and(|warning| warning.message.starts_with("/data/a.sha256 was replaced"))
    );

    // Nothing was published: an error that still names every recovery file.
    let refused = outcome(
        TransferPublication::RecoveryRequired {
            published: None,
            recovery: vec![kept("/data/.midnight-recovery-3/item")],
            error: failed("the exchange failed"),
        },
        0,
        true,
    )
    .err()
    .expect("an unpublished outcome is an error");
    assert!(
        refused
            .message
            .contains("Recovery file: /data/.midnight-recovery-3/item"),
        "{}",
        refused.message
    );
}

#[test]
fn a_new_file_is_reported_as_created() {
    let created = outcome(
        TransferPublication::PublishedDurabilityUnconfirmed {
            path: "/data/new.txt".into(),
            error: failed("sync failed"),
        },
        7,
        false,
    )
    .unwrap();
    assert_eq!(created.verified_bytes, 7);
    assert!(created.recovery.is_empty());
    assert!(
        created
            .warning
            .is_some_and(|warning| warning.message.starts_with("/data/new.txt was created"))
    );
}
