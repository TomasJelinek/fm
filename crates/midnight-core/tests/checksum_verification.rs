use midnight_core::*;

#[test]
fn verification_requests_validate_native_paths_ids_and_limits() {
    let mut request = ChecksumVerifyRequest {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(2),
        manifest: std::env::current_dir().unwrap().join("checksums.sha256"),
        limits: ChecksumLimits::default(),
    };
    assert!(request.validate().unwrap() > 0);
    request.job = ChecksumJobId(0);
    assert!(request.validate().is_err());
    request.job = ChecksumJobId(1);
    request.manifest = "relative.md5".into();
    assert!(request.validate().is_err());
}

#[test]
fn five_algorithms_keep_existing_discriminants_and_extensions() {
    for (index, (algorithm, extension, bytes)) in [
        (ChecksumAlgorithm::Md5, "md5", 16),
        (ChecksumAlgorithm::Sha1, "sha1", 20),
        (ChecksumAlgorithm::Sha256, "sha256", 32),
        (ChecksumAlgorithm::Sha512, "sha512", 64),
        (ChecksumAlgorithm::Crc32, "sfv", 4),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(algorithm as usize, index);
        assert_eq!(algorithm.extension(), extension);
        assert_eq!(algorithm.bytes(), bytes);
        assert!(ChecksumAlgorithms::all().contains(algorithm));
    }
    assert_eq!(ChecksumAlgorithm::ALL.len(), 5);
}

#[test]
fn only_complete_reads_expose_calculated_digests() {
    let mut row = ChecksumRow {
        id: ChecksumRowId(1),
        path: "file".into(),
        identity: None,
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 0,
        digests: vec![ChecksumDigest {
            algorithm: ChecksumAlgorithm::Crc32,
            bytes: vec![0; 4],
        }],
        error: None,
        retention: ChecksumRetention::new(()),
    };
    for state in [
        ChecksumRowState::Calculated,
        ChecksumRowState::Verified,
        ChecksumRowState::Mismatch,
    ] {
        row.state = state;
        assert!(row.digest(ChecksumAlgorithm::Crc32).is_some());
    }
    for state in [
        ChecksumRowState::Missing,
        ChecksumRowState::SourceChanged,
        ChecksumRowState::ReadError,
        ChecksumRowState::Cancelled,
    ] {
        row.state = state;
        assert!(row.digest(ChecksumAlgorithm::Crc32).is_none());
    }
}
