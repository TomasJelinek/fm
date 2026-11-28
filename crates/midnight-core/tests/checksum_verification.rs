use midnight_core::*;

#[test]
fn verification_requests_validate_native_paths_and_ids() {
    let mut request = ChecksumVerifyRequest {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(2),
        manifest: std::env::current_dir().unwrap().join("checksums.sha256"),
    };
    assert!(request.validate().is_ok());
    request.job = ChecksumJobId(0);
    assert!(request.validate().is_err());
    request.job = ChecksumJobId(1);
    request.manifest = "relative.md5".into();
    assert!(request.validate().is_err());
}

/// A calculation may name any number of targets: only malformed ones are refused.
#[test]
fn calculation_requests_take_any_number_of_targets() {
    let folder = std::env::current_dir().unwrap();
    let paths: Vec<_> = (0..5_000)
        .map(|index| folder.join(format!("{index}.bin")))
        .collect();
    let mut request = ChecksumRequest::new(ChecksumJobId(1), ChecksumGeneration(1), paths);
    assert!(
        request.validate().is_ok(),
        "more than the former 4,096 targets"
    );
    request
        .targets
        .push(ChecksumTarget::from(folder.join("..").join("x")));
    assert!(request.validate().is_err(), "traversal is refused");
}

/// Digests keep each algorithm's fixed length in one allocation, in the order of
/// `ChecksumAlgorithm::ALL` whatever order they were given in.
#[test]
fn digests_are_found_by_algorithm_in_canonical_order() {
    let sha256 = [0x11; 32];
    let crc32 = [0x22; 4];
    let md5 = [0x33; 16];
    let digests = ChecksumDigests::new(&[
        (ChecksumAlgorithm::Crc32, &crc32),
        (ChecksumAlgorithm::Sha256, &sha256),
        (ChecksumAlgorithm::Md5, &md5),
    ])
    .unwrap();
    let order: Vec<_> = digests.iter().map(|digest| digest.algorithm).collect();
    assert_eq!(
        order,
        [
            ChecksumAlgorithm::Md5,
            ChecksumAlgorithm::Sha256,
            ChecksumAlgorithm::Crc32
        ]
    );
    assert_eq!(
        digests.get(ChecksumAlgorithm::Sha256).unwrap().bytes,
        sha256
    );
    assert_eq!(digests.get(ChecksumAlgorithm::Crc32).unwrap().bytes, crc32);
    assert!(digests.get(ChecksumAlgorithm::Sha512).is_none());
    assert!(ChecksumDigests::new(&[(ChecksumAlgorithm::Md5, &[0; 15])]).is_none());
    assert!(
        ChecksumDigests::new(&[
            (ChecksumAlgorithm::Md5, &md5),
            (ChecksumAlgorithm::Md5, &md5)
        ])
        .is_none()
    );
    assert!(ChecksumDigests::default().is_empty());
}

fn row(id: u64, name: &str) -> ChecksumRow {
    ChecksumRow {
        id: ChecksumRowId(id),
        name: std::path::Path::new(name).into(),
        identity: None,
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 0,
        digests: ChecksumDigests::default(),
        error: None,
    }
}

/// Rows are shared in chunks; any row is found by its place or its ID without a scan.
#[test]
fn rows_are_found_across_chunks_by_place_and_id() {
    let base: std::sync::Arc<std::path::Path> = std::env::current_dir().unwrap().into();
    let retention = ChecksumRetention::new(());
    let mut rows = ChecksumRows::default();
    let mut next = 1;
    for size in [3, 0, 1, 5] {
        let chunk = (0..size)
            .map(|_| {
                next += 1;
                row(next - 1, &format!("f{}", next - 1))
            })
            .collect();
        rows.push(std::sync::Arc::new(ChecksumChunk {
            base: base.clone(),
            rows: chunk,
            retention: retention.clone(),
        }));
    }
    assert_eq!(rows.len(), 9);
    assert_eq!(rows.chunks().len(), 3, "an empty chunk is not kept");
    for index in 0..9 {
        let found = rows.get(index).unwrap();
        assert_eq!(found.id.0, index as u64 + 1);
        assert_eq!(found.path(), base.join(format!("f{}", index + 1)));
        assert_eq!(rows.by_id(index as u64 + 1).unwrap().id, found.id);
    }
    assert!(rows.get(9).is_none());
    assert!(rows.by_id(0).is_none());
    let ids: Vec<_> = rows.iter().map(|row| row.id.0).collect();
    assert_eq!(ids, (1..=9).collect::<Vec<_>>());
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
    let mut row = row(1, "file");
    row.digests = ChecksumDigests::new(&[(ChecksumAlgorithm::Crc32, &[0; 4])]).unwrap();
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
    let bytes: Vec<u8> = (0..=255).collect();
    let digest = ChecksumDigest {
        algorithm: ChecksumAlgorithm::Sha512,
        bytes: &bytes,
    };
    let expected: String = (0..=255_u8).map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest.hex(), expected);
    let mut appended = String::from("x");
    digest.push_hex(&mut appended);
    assert_eq!(appended, format!("x{expected}"));
    let empty = ChecksumDigest {
        algorithm: ChecksumAlgorithm::Crc32,
        bytes: &[],
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

/// The buffer encoder writes exactly the lines `checksum_file_line` returns, appends
/// nothing for a name it refuses, and names are found relative to the checksum file's
/// folder from a row's base folder and name.
#[test]
fn appended_lines_match_whole_lines_and_names_follow_the_folder() {
    use std::path::Path;
    let folder = std::env::current_dir().unwrap().join("výsledky");
    let bytes = [0xab; 32];
    let digest = ChecksumDigest {
        algorithm: ChecksumAlgorithm::Sha256,
        bytes: &bytes,
    };
    let crc = [0x01, 0x02, 0x03, 0x04];
    let sfv = ChecksumDigest {
        algorithm: ChecksumAlgorithm::Crc32,
        bytes: &crc,
    };
    let mut text = String::from("start\n");
    for (digest, name) in [
        (digest, "a b 文档.txt"),
        (digest, "two\nlines"),
        (sfv, "a.txt"),
        (digest, "sub/x.bin"),
    ] {
        let relative = Path::new(name);
        let whole = checksum_file_line(
            digest.algorithm,
            &digest.hex(),
            &folder.join(relative),
            &folder,
        )
        .unwrap();
        let before = text.len();
        push_checksum_file_line(&mut text, digest, relative).unwrap();
        assert_eq!(&text[before..], whole, "{name:?}");
    }
    let before = text.clone();
    assert!(push_checksum_file_line(&mut text, sfv, Path::new(";comment")).is_err());
    assert!(push_checksum_file_line(&mut text, digest, Path::new("")).is_err());
    assert_eq!(text, before, "a refused name appends nothing");
    // A row's name is below its base; the checksum file's folder may be the base, a
    // folder below it or above it.
    let base = folder.as_path();
    let name = Path::new("sub").join("x.bin");
    assert_eq!(checksum_file_name(base, &name, base).unwrap(), name);
    assert_eq!(
        checksum_file_name(base, &name, &base.join("sub")).unwrap(),
        Path::new("x.bin")
    );
    assert_eq!(
        checksum_file_name(base, &name, base.parent().unwrap()).unwrap(),
        Path::new("výsledky").join("sub").join("x.bin")
    );
    assert!(checksum_file_name(base, &name, &base.join("other")).is_none());
}

/// A row is small and keeps only its name (below its chunk's folder) and its digests on
/// the heap: no copy of the folder, no display text, no shared ownership of its own (its
/// chunk carries the folder and the job's ownership for all its rows).
#[test]
fn a_row_keeps_its_name_and_digests_only() {
    #[cfg(target_pointer_width = "64")]
    {
        // ID, name, identity, kind, state, bytes read, digests and error, as laid out.
        assert!(
            std::mem::size_of::<ChecksumRow>() <= 104,
            "{} bytes",
            std::mem::size_of::<ChecksumRow>()
        );
        assert!(std::mem::size_of::<ChecksumDigests>() <= 24);
        assert_eq!(std::mem::size_of::<Option<Box<FsError>>>(), 8);
    }
    let sha256 = [7; 32];
    let crc32 = [8; 4];
    let mut row = row(1, "sub/file.bin");
    row.digests = ChecksumDigests::new(&[
        (ChecksumAlgorithm::Sha256, &sha256),
        (ChecksumAlgorithm::Crc32, &crc32),
    ])
    .unwrap();
    // The digests are exactly their algorithms' lengths, together.
    let stored: usize = row.digests.iter().map(|digest| digest.bytes.len()).sum();
    assert_eq!(stored, 36);
    assert_eq!(row.name.as_os_str().len(), "sub/file.bin".len());
    assert!(row.error.is_none());
}

// The process's memory as each platform reports it, for the measurements below.
#[cfg(target_os = "linux")]
#[path = "support/process_memory_linux.rs"]
mod process_memory;
#[cfg(windows)]
#[path = "support/process_memory_windows.rs"]
mod process_memory;

/// Rows for this measurement: `MIDNIGHT_CHECKSUM_ROWS` (100,000 by default).
#[cfg(any(windows, target_os = "linux"))]
fn measured_rows() -> usize {
    std::env::var("MIDNIGHT_CHECKSUM_ROWS")
        .ok()
        .and_then(|rows| rows.parse().ok())
        .unwrap_or(100_000)
}

/// A typical row's folder (a ~75-byte absolute path) and its name below it (~25 bytes).
#[cfg(any(windows, target_os = "linux"))]
fn measured_paths(index: usize) -> (std::path::PathBuf, String) {
    (
        std::path::PathBuf::from(
            "C:\\Users\\someone\\Documents\\Projects\\photographs-2026\\holiday",
        ),
        format!("album {:02}\\IMG_{index:07}.jpg", index % 40),
    )
}

/// A measurement, not a check (run each alone, in release): the process's private memory
/// per row for the compact rows, kept in chunks of 64 as a run keeps them.
#[test]
#[cfg(any(windows, target_os = "linux"))]
#[ignore = "measures process memory; run alone with --ignored --nocapture"]
fn row_memory_compact() {
    let count = measured_rows();
    let retention = ChecksumRetention::new(());
    let before = process_memory::private_bytes();
    let (folder, _) = measured_paths(0);
    let base: std::sync::Arc<std::path::Path> = folder.into();
    let mut rows = ChecksumRows::default();
    let digest = [0x5a; 32];
    let mut batch = Vec::with_capacity(CHECKSUM_BATCH_SIZE);
    for index in 0..count {
        let (_, name) = measured_paths(index);
        batch.push(ChecksumRow {
            id: ChecksumRowId(index as u64 + 1),
            name: std::path::Path::new(&name).into(),
            identity: Some(TransferIdentity {
                device: 1,
                object: [1; 16],
            }),
            kind: Some(EntryKind::File),
            state: ChecksumRowState::Calculated,
            bytes_read: 4096,
            digests: ChecksumDigests::new(&[(ChecksumAlgorithm::Sha256, &digest)]).unwrap(),
            error: None,
        });
        if batch.len() == CHECKSUM_BATCH_SIZE || index + 1 == count {
            rows.push(std::sync::Arc::new(ChecksumChunk {
                base: base.clone(),
                rows: std::mem::take(&mut batch).into_boxed_slice(),
                retention: retention.clone(),
            }));
        }
    }
    let after = process_memory::private_bytes();
    assert_eq!(rows.len(), count);
    let total = after.saturating_sub(before);
    println!(
        "compact rows: {count} rows, {total} bytes, {} bytes per row (ChecksumRow is {} bytes)",
        total / count as u64,
        std::mem::size_of::<ChecksumRow>()
    );
}
