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

#[test]
fn sfv_export_places_crc_after_exact_unicode_filename() {
    let (mut run, mut request) = fixture("folder/č文 abc.txt");
    run.algorithms = ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap();
    request.algorithm = ChecksumAlgorithm::Crc32;
    Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]).digests = vec![ChecksumDigest {
        algorithm: ChecksumAlgorithm::Crc32,
        bytes: vec![0x35, 0x24, 0x41, 0xc2],
    }];
    let (bytes, _) = encode(
        &run,
        &request,
        request.destination.parent().unwrap(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "folder/č文 abc.txt  352441c2\n"
    );
}

#[test]
fn sfv_refuses_names_that_comments_or_whitespace_would_reinterpret() {
    for name in [
        ";comment",
        " #comment",
        " leading",
        "\tleading",
        "trailing ",
        "line\nbreak",
        "line\rbreak",
    ] {
        let (mut run, mut request) = fixture(name);
        run.algorithms = ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap();
        request.algorithm = ChecksumAlgorithm::Crc32;
        Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]).digests = vec![ChecksumDigest {
            algorithm: ChecksumAlgorithm::Crc32,
            bytes: vec![0; 4],
        }];
        assert!(
            encode(
                &run,
                &request,
                request.destination.parent().unwrap(),
                &Cancellation::default()
            )
            .is_err(),
            "{name:?}"
        );
    }
}

#[test]
fn verification_results_cannot_be_exported_as_a_calculation_manifest() {
    for state in [ChecksumRowState::Verified, ChecksumRowState::Mismatch] {
        let (mut run, request) = fixture("file");
        Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]).state = state;
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
}

#[test]
fn all_five_exports_round_trip_through_compatible_manifest_records() {
    for algorithm in ChecksumAlgorithm::ALL {
        let (mut run, mut request) = fixture("folder/č文 abc.txt");
        run.algorithms = ChecksumAlgorithms::new(&[algorithm]).unwrap();
        request.algorithm = algorithm;
        request.destination.set_extension(algorithm.extension());
        Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]).digests = vec![ChecksumDigest {
            algorithm,
            bytes: vec![0xab; algorithm.bytes()],
        }];
        let (bytes, _) = encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default(),
        )
        .unwrap();
        let parsed = super::super::format::parse_manifest(
            &bytes,
            &request.destination,
            ChecksumLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(parsed.algorithm, algorithm);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, run.rows[0].path);
        assert_eq!(parsed.entries[0].digest, vec![0xab; algorithm.bytes()]);
    }
}

#[test]
fn exports_preflight_the_same_budgets_and_name_ambiguity_as_verification() {
    let (mut run, request) = fixture("first");
    // A successful calculation can occupy nearly its complete retained budget.
    // The exported text also counts against parsing capacity; never publish a file
    // that the verifier will refuse after the user has already saved it.
    let count = ChecksumLimits::default().max_retained_bytes
        / (run.rows[0].path.as_os_str().as_encoded_bytes().len() * 4 + 1024);
    run.rows = (0..count)
        .map(|index| {
            let mut row = (*run.rows[0]).clone();
            row.id = ChecksumRowId(index as u64 + 1);
            row.path.set_file_name(format!("entry-{index:05}"));
            Arc::new(row)
        })
        .collect();
    assert!(
        encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default()
        )
        .is_err()
    );

    let (mut run, request) = fixture("file");
    let mut alias = (*run.rows[0]).clone();
    alias.path.set_file_name("FILE");
    run.rows = vec![run.rows[0].clone(), Arc::new(alias)].into();
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
fn sfv_export_refuses_a_bsd_lookalike_that_would_select_a_different_filename() {
    let (mut run, mut request) = fixture("CRC32 (different.txt) =");
    run.algorithms = ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap();
    request.algorithm = ChecksumAlgorithm::Crc32;
    request.destination.set_extension("sfv");
    Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]).digests = vec![ChecksumDigest {
        algorithm: ChecksumAlgorithm::Crc32,
        bytes: vec![0; 4],
    }];
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
