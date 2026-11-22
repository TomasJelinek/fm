use super::*;

fn row(id: u64, name: &str, state: ChecksumRowState) -> ChecksumRow {
    let digest = [id as u8; 32];
    ChecksumRow {
        id: ChecksumRowId(id),
        name: Path::new(name).into(),
        identity: None,
        kind: Some(EntryKind::File),
        state,
        bytes_read: 1,
        digests: if state == ChecksumRowState::Calculated {
            ChecksumDigests::new(&[(ChecksumAlgorithm::Sha256, &digest)]).unwrap()
        } else {
            ChecksumDigests::default()
        },
        error: None,
    }
}

/// Rows 1…`count` (r1.bin…) in chunks of 64 below `base`; `failed` ones could not be read.
fn rows(base: &Path, count: u64, failed: &[u64]) -> ChecksumRows {
    let retention = ChecksumRetention::new(());
    let mut rows = ChecksumRows::default();
    let all: Vec<_> = (1..=count)
        .map(|id| {
            let state = if failed.contains(&id) {
                ChecksumRowState::ReadError
            } else {
                ChecksumRowState::Calculated
            };
            row(id, &format!("sub/r{id}.bin"), state)
        })
        .collect();
    for chunk in all.chunks(CHECKSUM_BATCH_SIZE) {
        rows.push(Arc::new(ChecksumChunk {
            base: base.into(),
            rows: chunk.to_vec().into_boxed_slice(),
            retention: retention.clone(),
        }));
    }
    rows
}

/// The selection bits of these row IDs and waiting listed places.
fn bits(ids: &[u64], waiting: &[u64]) -> ChecksumCopyRows {
    let slots = ids
        .iter()
        .map(|id| (id - 1) * 2)
        .chain(waiting.iter().map(|place| place * 2 + 1));
    let mut words = vec![0u64; 0];
    for slot in slots {
        let word = (slot / 64) as usize;
        if words.len() <= word {
            words.resize(word + 1, 0);
        }
        words[word] |= 1 << (slot % 64);
    }
    ChecksumCopyRows::Selected(words.into())
}

fn ample(text: &mut String, additional: usize) -> Result<(), std::collections::TryReserveError> {
    text.try_reserve_exact(additional)
}

fn hash(id: u64) -> String {
    format!("{:02x}", id as u8).repeat(32)
}

/// A copy takes the selected rows in the order shown, as hashes, full rows or
/// checksum-file lines named relative to the folder, and counts what it leaves out:
/// failed rows and listed files still waiting.
#[test]
fn a_copy_takes_the_selected_rows_in_order_and_counts_what_it_leaves_out() {
    let base = Path::new(if cfg!(windows) { "C:\\data" } else { "/data" });
    let rows = rows(base, 130, &[65]);
    // Rows across three chunks, one failed, and two listed files still waiting.
    let selection = bits(&[2, 65, 70, 129], &[3, 9]);
    let cancel = Cancellation::default();
    let sha = ChecksumAlgorithm::Sha256;
    let (text, report) = checksum_copy_text(
        &rows,
        &selection,
        ChecksumCopyKind::Hashes(sha),
        base,
        &cancel,
        &ample,
    )
    .unwrap();
    assert_eq!(text, format!("{}\n{}\n{}", hash(2), hash(70), hash(129)));
    assert_eq!(report.copied, 3);
    assert_eq!(
        report.left_out,
        [(CHECKSUM_COPY_FAILED, 1), (CHECKSUM_COPY_WAITING, 2)]
    );
    let (text, _) = checksum_copy_text(
        &rows,
        &selection,
        ChecksumCopyKind::Lines(sha),
        base,
        &cancel,
        &ample,
    )
    .unwrap();
    assert_eq!(
        text,
        format!(
            "{} *sub/r2.bin\n{} *sub/r70.bin\n{} *sub/r129.bin\n",
            hash(2),
            hash(70),
            hash(129)
        )
    );
    let (text, _) = checksum_copy_text(
        &rows,
        &ChecksumCopyRows::Row(70),
        ChecksumCopyKind::Rows,
        base,
        &cancel,
        &ample,
    )
    .unwrap();
    assert_eq!(
        text,
        // The row's own path: its folder joined with its name as it was found.
        format!(
            "{}\nSHA-256: {}",
            base.join("sub/r70.bin").display(),
            hash(70)
        )
    );
}

/// A copy larger than the former 4 MiB clipboard budget is built whole.
#[test]
fn a_copy_of_more_than_four_mebibytes_is_built() {
    let base = Path::new(if cfg!(windows) { "C:\\data" } else { "/data" });
    let count = 60_000;
    let rows = rows(base, count, &[]);
    let all: Vec<u64> = (1..=count).collect();
    let copy = ChecksumCopyText::new(
        rows,
        bits(&all, &[]),
        ChecksumCopyKind::Lines(ChecksumAlgorithm::Sha256),
        base.to_path_buf(),
    );
    copy.prepare(&Cancellation::default()).unwrap();
    let text = copy.take_text().unwrap();
    assert!(text.len() > 4 * 1024 * 1024, "{} bytes", text.len());
    assert_eq!(text.lines().count(), count as usize);
    assert_eq!(copy.report().unwrap().copied, count);
}

/// A copy that finds too little memory for its text fails with a plain error, keeps
/// nothing and never ends the process.
#[test]
fn a_copy_short_of_memory_fails_with_an_error() {
    let base = Path::new(if cfg!(windows) { "C:\\data" } else { "/data" });
    let copy = ChecksumCopyText::new(
        rows(base, 10, &[]),
        bits(&[1, 2, 3], &[]),
        ChecksumCopyKind::Hashes(ChecksumAlgorithm::Sha256),
        base.to_path_buf(),
    );
    let refused = |text: &mut String, _: usize| text.try_reserve(usize::MAX);
    let error = copy.build(&Cancellation::default(), &refused).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert_eq!(
        error.to_string(),
        "There is not enough memory to copy 3 rows."
    );
    assert!(copy.take_text().is_none());
    assert!(copy.report().is_none());
}

/// A copy that adds nothing fails (the clipboard keeps what it holds) but says why;
/// a cancelled one stops.
#[test]
fn a_copy_of_nothing_or_a_cancelled_copy_writes_nothing() {
    let base = Path::new(if cfg!(windows) { "C:\\data" } else { "/data" });
    let copy = ChecksumCopyText::new(
        rows(base, 4, &[2]),
        bits(&[2], &[7]),
        ChecksumCopyKind::Lines(ChecksumAlgorithm::Sha256),
        base.to_path_buf(),
    );
    let error = copy.prepare(&Cancellation::default()).unwrap_err();
    assert_eq!(error.to_string(), CHECKSUM_COPY_NOTHING);
    assert!(copy.take_text().is_none());
    assert_eq!(
        copy.report().unwrap().left_out,
        [(CHECKSUM_COPY_FAILED, 1), (CHECKSUM_COPY_WAITING, 1)]
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    let copy = ChecksumCopyText::new(
        rows(base, 4, &[]),
        bits(&[1, 2], &[]),
        ChecksumCopyKind::Hashes(ChecksumAlgorithm::Sha256),
        base.to_path_buf(),
    );
    assert_eq!(
        copy.prepare(&cancel).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}
