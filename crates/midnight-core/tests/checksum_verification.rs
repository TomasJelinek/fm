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

#[test]
fn digest_hex_is_two_lowercase_digits_per_byte_in_order() {
    let digest = ChecksumDigest {
        algorithm: ChecksumAlgorithm::Sha512,
        bytes: (0..=255).collect(),
    };
    let expected: String = (0..=255_u8).map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest.hex(), expected);
    let empty = ChecksumDigest {
        algorithm: ChecksumAlgorithm::Crc32,
        bytes: Vec::new(),
    };
    assert_eq!(empty.hex(), "");
}

#[test]
fn checksum_file_lines_name_files_below_the_folder_as_a_saved_file_does() {
    let folder = std::env::current_dir().unwrap().join("výsledky");
    let hex = "ab".repeat(32);
    let line =
        |algorithm, name: &str| checksum_file_line(algorithm, &hex, &folder.join(name), &folder);
    assert_eq!(
        line(ChecksumAlgorithm::Sha256, "a b 文档.txt").unwrap(),
        format!("{hex} *a b 文档.txt\n")
    );
    let nested = folder.join("sub").join("x.bin");
    assert_eq!(
        checksum_file_line(ChecksumAlgorithm::Sha256, &hex, &nested, &folder).unwrap(),
        format!("{hex} *sub/x.bin\n"),
        "names use / below the folder"
    );
    // A name with a line break is escaped, with the GNU marker before the hash.
    assert_eq!(
        line(ChecksumAlgorithm::Sha256, "two\nlines").unwrap(),
        format!("\\{hex} *two\\nlines\n")
    );
    assert_eq!(
        line(ChecksumAlgorithm::Crc32, "a.txt").unwrap(),
        format!("a.txt  {hex}\n"),
        "CRC32 lines are SFV"
    );
    assert!(line(ChecksumAlgorithm::Crc32, ";comment").is_err());
    let outside = std::env::current_dir().unwrap().join("jinde.txt");
    assert!(checksum_file_line(ChecksumAlgorithm::Sha256, &hex, &outside, &folder).is_err());
    assert!(checksum_file_line(ChecksumAlgorithm::Sha256, &hex, &folder, &folder).is_err());
}
