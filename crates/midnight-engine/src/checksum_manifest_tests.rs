use super::*;
use std::path::PathBuf;

fn root() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("_build/virtual-manifest")
}
/// Rows in one chunk below `base`.
fn chunk(base: &Path, rows: Vec<ChecksumRow>) -> ChecksumRows {
    Arc::new(ChecksumChunk {
        base: base.into(),
        rows: rows.into(),
        retention: ChecksumRetention::new(()),
    })
    .into()
}
/// Replace the run's rows with these, below the same base.
fn set_rows(run: &mut ChecksumRun, rows: Vec<ChecksumRow>) {
    let base = run.rows.chunks()[0].base.clone();
    run.rows = chunk(&base, rows);
}
fn digests(algorithm: ChecksumAlgorithm, bytes: &[u8]) -> ChecksumDigests {
    ChecksumDigests::new(&[(algorithm, bytes)]).unwrap()
}

fn fixture(name: &str) -> (ChecksumRun, ChecksumManifestRequest) {
    let root = root();
    let row = ChecksumRow {
        id: ChecksumRowId(1),
        name: Path::new(name).into(),
        identity: Some(TransferIdentity {
            device: 1,
            object: [1; 16],
        }),
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 0,
        digests: digests(ChecksumAlgorithm::Sha256, &[0; 32]),
        error: None,
    };
    (
        ChecksumRun {
            job: ChecksumJobId(1),
            generation: ChecksumGeneration(1),
            algorithms: ChecksumAlgorithms::default(),
            captured_roots: vec![root.join(name)].into(),
            rows: chunk(&root, vec![row]),
            retention: ChecksumRetention::new(()),
            summary: ChecksumSummary {
                completion: ChecksumCompletion::Completed,
                progress: ChecksumProgress::default(),
                diagnostics: Vec::new(),
            },
        },
        ChecksumManifestRequest {
            algorithm: ChecksumAlgorithm::Sha256,
            destination: root.join("out.sha256"),
            acknowledge_partial: None,
            overwrite: false,
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
fn incomplete_discovery_cannot_look_like_a_complete_manifest() {
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
    let mut row = run.rows[0].clone();
    row.digests = digests(ChecksumAlgorithm::Crc32, &[0x35, 0x24, 0x41, 0xc2]);
    set_rows(&mut run, vec![row]);
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
        let mut row = run.rows[0].clone();
        row.digests = digests(ChecksumAlgorithm::Crc32, &[0; 4]);
        set_rows(&mut run, vec![row]);
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
        let mut row = run.rows[0].clone();
        row.state = state;
        set_rows(&mut run, vec![row]);
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
        let mut row = run.rows[0].clone();
        row.digests = digests(algorithm, &vec![0xab; algorithm.bytes()]);
        set_rows(&mut run, vec![row]);
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
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(parsed.algorithm, algorithm);
        assert_eq!(parsed.names.len(), 1);
        assert_eq!(&*parsed.names[0], Path::new("folder/č文 abc.txt"));
        assert_eq!(parsed.digest(0).unwrap(), vec![0xab; algorithm.bytes()]);
    }
}

/// Never publish a file that the verifier will refuse after the user saved it: two names
/// that differ only in letter case are one name to it.
#[test]
fn exports_refuse_names_that_differ_only_in_letter_case() {
    let (mut run, request) = fixture("file");
    let mut alias = run.rows[0].clone();
    alias.id = ChecksumRowId(2);
    alias.name = Path::new("FILE").into();
    let first = run.rows[0].clone();
    set_rows(&mut run, vec![first, alias]);
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

/// A manifest has no size limit: a run of more rows than the former 16,384, with more text
/// than the former 16 MiB or ordinary names, is encoded, every row once, and reads back.
#[test]
fn exports_of_any_size_are_encoded_and_read_back() {
    const ROWS: usize = 20_000;
    for (padding, large) in [("n".repeat(800), true), ("x".to_owned(), false)] {
        let (mut run, request) = fixture("file");
        let template = run.rows[0].clone();
        let rows = (0..ROWS)
            .map(|index| {
                let mut row = template.clone();
                row.id = ChecksumRowId(index as u64 + 1);
                row.name = Path::new("folder")
                    .join(format!("{index:05}-{padding}.bin"))
                    .into();
                row
            })
            .collect();
        set_rows(&mut run, rows);
        run.summary.progress.calculated = ROWS as u64;
        let (bytes, counts) = encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(counts.included, ROWS as u64);
        assert_eq!(bytes.len() > 16 * 1024 * 1024, large);
        let lines = bytes.iter().filter(|byte| **byte == b'\n').count();
        assert_eq!(lines, ROWS);
    }
}

#[test]
fn sfv_export_refuses_a_bsd_lookalike_that_would_select_a_different_filename() {
    let (mut run, mut request) = fixture("CRC32 (different.txt) =");
    run.algorithms = ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap();
    request.algorithm = ChecksumAlgorithm::Crc32;
    request.destination.set_extension("sfv");
    let mut row = run.rows[0].clone();
    row.digests = digests(ChecksumAlgorithm::Crc32, &[0; 4]);
    set_rows(&mut run, vec![row]);
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
