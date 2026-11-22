//! The text of a copy of checksum rows. The window sends a [`ChecksumCopyText`] to the
//! clipboard worker, which builds the text there (never on the UI thread) from the run's
//! shared, immutable row chunks and a snapshot of the selection, then writes it.
use super::*;
use crate::{Cancellation, GeneratedTextSource};
use std::{
    collections::TryReserveError,
    fmt::Write as _,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// What a copy of rows writes for each of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumCopyKind {
    /// The checksum of the algorithm, one per line (Ctrl+C).
    Hashes(ChecksumAlgorithm),
    /// The path and every checksum, with a blank line between rows (Ctrl+Shift+C).
    Rows,
    /// The checksum-file line, exactly as Save manifest… writes it (Copy).
    Lines(ChecksumAlgorithm),
}

/// Which rows a copy takes, in the order shown.
#[derive(Clone, Debug)]
pub enum ChecksumCopyRows {
    /// A snapshot of the selection's bits: a row's slot is twice its ID less one, and an
    /// odd slot is a listed file still waiting for its row.
    Selected(Arc<[u64]>),
    /// The row with this ID alone (the context menu of a row outside the selection).
    Row(u64),
    /// A listed file alone, still waiting for its row.
    Waiting,
}

/// Why a selected row adds nothing to a copy: a listed file not read yet, a file that
/// could not be read (or changed, or is missing), skipped or cancelled, without a
/// checksum of the chosen algorithm, or a verification's result (a checksum file takes
/// calculated rows only). A name a checksum file cannot hold gives the encoder's reason.
pub const CHECKSUM_COPY_WAITING: &str = "not calculated yet";
pub const CHECKSUM_COPY_FAILED: &str = "failed";
pub const CHECKSUM_COPY_SKIPPED: &str = "skipped";
pub const CHECKSUM_COPY_CANCELLED: &str = "cancelled";
pub const CHECKSUM_COPY_NO_DIGEST: &str = "without the chosen checksum";
pub const CHECKSUM_COPY_VERIFICATION: &str = "verification result";
/// The error of a copy that adds nothing: the clipboard keeps what it holds.
pub const CHECKSUM_COPY_NOTHING: &str = "None of the selected rows adds anything to this copy.";

/// How many rows a copy took, and how many it left out for each reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChecksumCopyReport {
    pub copied: u64,
    pub left_out: Vec<(&'static str, u64)>,
}

impl ChecksumCopyReport {
    fn leave_out(&mut self, reason: &'static str, count: u64) {
        if count == 0 {
            return;
        }
        match self.left_out.iter_mut().find(|(known, _)| *known == reason) {
            Some((_, total)) => *total += count,
            None => self.left_out.push((reason, count)),
        }
    }
}

/// Why `row` adds nothing to a copy of this kind; none when it adds something.
pub fn checksum_copy_left_out(row: &ChecksumRow, kind: ChecksumCopyKind) -> Option<&'static str> {
    match row.state {
        ChecksumRowState::ReadError
        | ChecksumRowState::SourceChanged
        | ChecksumRowState::Missing => Some(CHECKSUM_COPY_FAILED),
        ChecksumRowState::Skipped => Some(CHECKSUM_COPY_SKIPPED),
        ChecksumRowState::Cancelled => Some(CHECKSUM_COPY_CANCELLED),
        ChecksumRowState::Verified | ChecksumRowState::Mismatch
            if matches!(kind, ChecksumCopyKind::Lines(_)) =>
        {
            Some(CHECKSUM_COPY_VERIFICATION)
        }
        _ => match kind {
            ChecksumCopyKind::Hashes(algorithm) | ChecksumCopyKind::Lines(algorithm)
                if row.digest(algorithm).is_none() =>
            {
                Some(CHECKSUM_COPY_NO_DIGEST)
            }
            ChecksumCopyKind::Rows if row.digests.is_empty() => Some(CHECKSUM_COPY_NO_DIGEST),
            _ => None,
        },
    }
}

/// Room for a text of `additional` more bytes, or an error when memory is short.
pub type ChecksumCopyReserve = dyn Fn(&mut String, usize) -> Result<(), TryReserveError>;

/// The error of a copy of `rows` rows that found too little memory.
fn short_of_memory(rows: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        format!(
            "There is not enough memory to copy {rows} {}.",
            if rows == 1 { "row" } else { "rows" }
        ),
    )
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Checksum copy cancelled.")
}

/// Build the text of a copy in one pass into a text sized for it, in the order shown,
/// and say what it left out. Names in checksum-file lines are relative to `folder`.
/// `reserve` makes room for the text (`String::try_reserve_exact` outside tests); a copy
/// that finds too little memory fails with an error instead of ending the process.
pub fn checksum_copy_text(
    rows: &ChecksumRows,
    selection: &ChecksumCopyRows,
    kind: ChecksumCopyKind,
    folder: &Path,
    cancel: &Cancellation,
    reserve: &ChecksumCopyReserve,
) -> io::Result<(String, ChecksumCopyReport)> {
    let mut report = ChecksumCopyReport::default();
    let mut chosen: Vec<ChecksumRowRef<'_>> = Vec::new();
    let mut waiting = 0u64;
    match selection {
        ChecksumCopyRows::Selected(bits) => {
            const EVEN: u64 = 0x5555_5555_5555_5555;
            let selected: u64 = bits
                .iter()
                .map(|word| u64::from((word & EVEN).count_ones()))
                .sum();
            chosen
                .try_reserve_exact(usize::try_from(selected).unwrap_or(usize::MAX))
                .map_err(|_| short_of_memory(selected))?;
            for (index, word) in bits.iter().enumerate() {
                if index.is_multiple_of(1024) && cancel.is_cancelled() {
                    return Err(cancelled());
                }
                let mut remaining = *word;
                while remaining != 0 {
                    let slot = index * 64 + remaining.trailing_zeros() as usize;
                    remaining &= remaining - 1;
                    if !slot.is_multiple_of(2) {
                        waiting += 1;
                    } else if let Some(row) = rows.by_id(slot as u64 / 2 + 1) {
                        chosen.push(row);
                    }
                }
            }
        }
        ChecksumCopyRows::Row(id) => chosen.extend(rows.by_id(*id)),
        ChecksumCopyRows::Waiting => waiting = 1,
    }
    let capacity = chosen.iter().fold(0usize, |total, row| {
        total.saturating_add(match kind {
            ChecksumCopyKind::Hashes(algorithm) => algorithm.bytes() * 2 + 1,
            ChecksumCopyKind::Lines(algorithm) => {
                algorithm.bytes() * 2 + 4 + row.name.as_os_str().len()
            }
            ChecksumCopyKind::Rows => {
                row.base.as_os_str().len()
                    + row.name.as_os_str().len()
                    + row
                        .digests
                        .iter()
                        .map(|digest| digest.bytes.len() * 2 + 10)
                        .sum::<usize>()
                    + 3
            }
        })
    });
    let mut text = String::new();
    reserve(&mut text, capacity).map_err(|_| short_of_memory(chosen.len() as u64))?;
    for (index, row) in chosen.iter().enumerate() {
        if index.is_multiple_of(4096) && cancel.is_cancelled() {
            return Err(cancelled());
        }
        if let Some(reason) = checksum_copy_left_out(row, kind) {
            report.leave_out(reason, 1);
            continue;
        }
        match kind {
            ChecksumCopyKind::Hashes(algorithm) => {
                let Some(digest) = row.digest(algorithm) else {
                    continue;
                };
                if report.copied > 0 {
                    text.push('\n');
                }
                digest.push_hex(&mut text);
            }
            ChecksumCopyKind::Rows => {
                if report.copied > 0 {
                    text.push_str("\n\n");
                }
                let _ = write!(text, "{}", row.path().display());
                for digest in row.digests.iter() {
                    text.push('\n');
                    text.push_str(digest.algorithm.label());
                    text.push_str(": ");
                    digest.push_hex(&mut text);
                }
            }
            ChecksumCopyKind::Lines(algorithm) => {
                let Some(digest) = row.digest(algorithm) else {
                    continue;
                };
                let Some(name) = checksum_file_name(row.base, &row.name, folder) else {
                    report.leave_out(CHECKSUM_NAME_NOT_BELOW, 1);
                    continue;
                };
                if let Err((_, reason)) = push_checksum_file_line(&mut text, digest, &name) {
                    report.leave_out(reason, 1);
                    continue;
                }
            }
        }
        report.copied += 1;
    }
    // Selected files still waiting for their rows come after the rows shown.
    report.leave_out(CHECKSUM_COPY_WAITING, waiting);
    Ok((text, report))
}

/// A copy of checksum rows that the clipboard worker builds and writes: the row chunks
/// are shared with the window, the selection is a snapshot of its bits, so asking for a
/// copy costs the UI thread no more than copying those bits.
#[derive(Debug)]
pub struct ChecksumCopyText {
    rows: ChecksumRows,
    selection: ChecksumCopyRows,
    kind: ChecksumCopyKind,
    folder: PathBuf,
    text: Mutex<Option<String>>,
    report: Mutex<Option<ChecksumCopyReport>>,
}

impl ChecksumCopyText {
    pub fn new(
        rows: ChecksumRows,
        selection: ChecksumCopyRows,
        kind: ChecksumCopyKind,
        folder: PathBuf,
    ) -> Self {
        Self {
            rows,
            selection,
            kind,
            folder,
            text: Mutex::new(None),
            report: Mutex::new(None),
        }
    }
    pub fn kind(&self) -> ChecksumCopyKind {
        self.kind
    }
    /// What the copy took and left out, once the worker built it.
    pub fn report(&self) -> Option<ChecksumCopyReport> {
        self.report.lock().ok()?.clone()
    }
    /// Build the text with `reserve` making room for it (see [`checksum_copy_text`]).
    pub fn build(&self, cancel: &Cancellation, reserve: &ChecksumCopyReserve) -> io::Result<()> {
        let (text, report) = checksum_copy_text(
            &self.rows,
            &self.selection,
            self.kind,
            &self.folder,
            cancel,
            reserve,
        )?;
        let copied = report.copied;
        if let Ok(mut slot) = self.report.lock() {
            *slot = Some(report);
        }
        if copied == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                CHECKSUM_COPY_NOTHING,
            ));
        }
        if let Ok(mut slot) = self.text.lock() {
            *slot = Some(text);
        }
        Ok(())
    }
}

impl GeneratedTextSource for ChecksumCopyText {
    fn prepare(&self, cancel: &Cancellation) -> io::Result<()> {
        self.build(cancel, &|text, additional| {
            text.try_reserve_exact(additional)
        })
    }
    fn take_text(&self) -> Option<String> {
        self.text.lock().ok()?.take()
    }
    fn length(&self) -> u64 {
        self.text
            .lock()
            .ok()
            .and_then(|text| text.as_ref().map(|text| text.len() as u64))
            .unwrap_or(0)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let text = self
            .text
            .lock()
            .map_err(|_| io::Error::other("The checksum copy is unavailable."))?;
        let bytes = text.as_deref().unwrap_or_default().as_bytes();
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let count = output.len().min(bytes.len() - start);
        output[..count].copy_from_slice(&bytes[start..start + count]);
        Ok(count)
    }
}

#[cfg(test)]
#[path = "checksum_copy_tests.rs"]
mod tests;
