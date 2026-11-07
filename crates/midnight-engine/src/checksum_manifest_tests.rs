use super::*;

fn fixture(name: &str) -> (ChecksumRun, ChecksumManifestRequest) {
    let root = std::env::current_dir()
        .unwrap()
        .join("_build/virtual-manifest");
    let retention = ChecksumRetention::new(());
    let row = Arc::new(ChecksumRow {
        id: ChecksumRowId(1),
        path: root.join(name),
        identity: Some(TransferIdentity {
            device: 1,
            object: [1; 16],
        }),
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 0,
        digests: vec![ChecksumDigest {
            algorithm: ChecksumAlgorithm::Sha256,
            bytes: vec![0; 32],
        }],
        error: None,
        retention: retention.clone(),
    });
    (
        ChecksumRun {
            job: ChecksumJobId(1),
            generation: ChecksumGeneration(1),
            algorithms: ChecksumAlgorithms::default(),
            captured_roots: vec![row.path.clone()].into(),
            rows: vec![row].into(),
            retention,
            summary: ChecksumSummary {
                completion: ChecksumCompletion::Completed,
                progress: ChecksumProgress::default(),
                reached_budgets: Vec::new(),
                diagnostics: Vec::new(),
            },
        },
        ChecksumManifestRequest {
            algorithm: ChecksumAlgorithm::Sha256,
            destination: root.join("out.sha256"),
            acknowledge_partial: None,
        },
    )
}

#[test]
fn gnu_binary_records_escape_control_characters_without_bom_or_lossy_names() {
    let (run, request) = fixture("line\ncarriage\rč文");
    let (bytes, counts) = encode(
        &run,
        &request,
        request.destination.parent().unwrap(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(counts.included, 1);
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        format!("\\{} *line\\ncarriage\\rč文\n", "0".repeat(64))
    );
}

#[test]
fn incomplete_discovery_and_bad_digest_length_cannot_look_like_a_complete_manifest() {
    let (mut run, mut request) = fixture("file");
    run.summary.completion = ChecksumCompletion::Incomplete;
    assert!(
        encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default()
        )
        .is_err()
    );
    request.acknowledge_partial = Some(ChecksumPartialAcknowledgement {
        included: 1,
        excluded: 0,
        incomplete_discovery: true,
    });
    assert!(
        encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default()
        )
        .is_ok()
    );
    let row = Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]);
    row.digests[0].bytes.pop();
    assert!(
        encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default()
        )
        .is_err()
    );
}

#[test]
fn source_outside_manifest_directory_and_cancelled_encoding_are_refused() {
    let (run, request) = fixture("file");
    assert!(
        encode(
            &run,
            &request,
            &request.destination.parent().unwrap().join("child"),
            &Cancellation::default()
        )
        .is_err()
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &cancel
        )
        .unwrap_err()
        .kind,
        io::ErrorKind::Interrupted
    );
}

#[cfg(unix)]
#[path = "checksum_manifest_unix_tests.rs"]
mod unix;
#[cfg(windows)]
#[path = "checksum_manifest_windows_tests.rs"]
mod windows;
