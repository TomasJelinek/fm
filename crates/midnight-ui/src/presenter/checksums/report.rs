//! What a checksum window says beyond its result rows: the issue line, the save line
//! and the details behind both. Paths keep their own lines (see `append_path`).
use super::*;
use crate::error_presentation::{append_path, describe};
use crate::job_presentation::counted;

/// Why a selected row adds nothing to a copy (`midnight_core::checksum_copy_left_out`,
/// which the clipboard worker applies while it builds the copy).
pub(super) const WAITING: &str = CHECKSUM_COPY_WAITING;
pub(super) const FAILED: &str = CHECKSUM_COPY_FAILED;
pub(super) const SKIPPED: &str = CHECKSUM_COPY_SKIPPED;
pub(super) const CANCELLED: &str = CHECKSUM_COPY_CANCELLED;
pub(super) const NO_DIGEST: &str = CHECKSUM_COPY_NO_DIGEST;
pub(super) const VERIFICATION: &str = CHECKSUM_COPY_VERIFICATION;

/// Each reason a selected row is left out of a copy (above), and each reason
/// `checksum_file_line` gives for a name it cannot write, as the words that follow a count
/// of files, singular and plural. Other reasons keep their own text.
const LEFT_OUT: [(&str, &str, &str); 9] = [
    (
        WAITING,
        "file not calculated yet",
        "files not calculated yet",
    ),
    (FAILED, "failed file", "failed files"),
    (SKIPPED, "skipped file", "skipped files"),
    (CANCELLED, "cancelled file", "cancelled files"),
    (
        NO_DIGEST,
        "file without the chosen checksum",
        "files without the chosen checksum",
    ),
    (VERIFICATION, "verification result", "verification results"),
    (
        "must be below its containing directory",
        "file not below the checksum file's folder",
        "files not below the checksum file's folder",
    ),
    (
        "cannot round-trip through the UTF-8",
        "file whose name is not valid Unicode",
        "files whose names are not valid Unicode",
    ),
    (
        "unambiguously in SFV format",
        "file whose name SFV cannot hold unambiguously",
        "files whose names SFV cannot hold unambiguously",
    ),
];

impl CopyNote {
    /// What the copy did, then how many files it left out for each reason.
    pub(crate) fn text(&self, locale: &NumberLocale) -> String {
        let (one, many) = if self.rows {
            ("row", "rows")
        } else {
            ("checksum", "checksums")
        };
        let copied = match self.copied {
            0 => "Nothing copied".to_owned(),
            copied => format!("Copied {}", counted(copied, one, many, locale)),
        };
        let left_out = self
            .left_out
            .iter()
            .map(|(reason, count)| {
                match LEFT_OUT.iter().find(|(cause, ..)| reason.contains(cause)) {
                    Some((_, one, many)) => counted(*count, one, many, locale),
                    None => format!(
                        "{}: {}",
                        counted(*count, "file", "files", locale),
                        reason.trim_end_matches('.')
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("{copied}. Left out: {left_out}.")
    }
}

impl ChecksumSession {
    /// The attention line: the window's error, else the first backend diagnostic of the
    /// finished run (a refused manifest, an unreadable source).
    pub(crate) fn issue(&self) -> Option<String> {
        if let Some(line) = self.error.lines().find(|line| !line.trim().is_empty()) {
            return Some(line.to_owned());
        }
        let summary = &self.run.as_ref()?.summary;
        let first = summary.diagnostics.first()?;
        let line = first.message.lines().next().unwrap_or_default();
        Some(match summary.diagnostics.len() - 1 {
            0 => line.to_owned(),
            more => format!("{line} (and {more} more)"),
        })
    }

    /// The save line: what a running save does, else what the last Copy left out, else
    /// where the last save went.
    pub(crate) fn save_status(&self, locale: &NumberLocale) -> String {
        if self.save.is_some() {
            return self.save_message.clone();
        }
        if let Some(note) = &self.copy_note {
            return note.text(locale);
        }
        let Some(saved) = &self.saved else {
            return String::new();
        };
        let name = saved
            .path
            .file_name()
            .unwrap_or(saved.path.as_os_str())
            .to_string_lossy();
        let mut line = format!(
            "Saved {name} · {} included",
            locale.format_u64(saved.included)
        );
        if saved.excluded > 0 {
            line.push_str(&format!(
                " · {} excluded",
                locale.format_u64(saved.excluded)
            ));
        }
        if !saved.recovery.is_empty() {
            line.push_str(" · the replaced file is kept for recovery");
        }
        line
    }

    /// Everything the two lines summarize, for the Details view and Copy details.
    pub(crate) fn details(&self, locale: &NumberLocale) -> String {
        fn section(text: &mut String, heading: &str) {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(heading);
        }
        let mut text = String::new();
        if !self.error.is_empty() {
            section(&mut text, &self.error);
        }
        if let Some(saved) = &self.saved {
            section(
                &mut text,
                &format!(
                    "Saved and verified {} bytes\n",
                    locale.format_u64(saved.verified_bytes)
                ),
            );
            append_path(&mut text, &saved.path);
            text.push_str(&format!(
                "\n{} included · {} excluded",
                locale.format_u64(saved.included),
                locale.format_u64(saved.excluded)
            ));
            if !saved.recovery.is_empty() {
                text.push_str("\n\nThe replaced file is kept for recovery at");
                for path in &saved.recovery {
                    text.push('\n');
                    append_path(&mut text, path);
                }
                text.push_str("\nDelete it when you no longer need it.");
            }
        }
        if let Some(note) = &self.copy_note {
            section(&mut text, &note.text(locale));
        }
        if let Some(manifest) = &self.verification {
            section(&mut text, "Checksum file\n");
            append_path(&mut text, manifest);
        }
        section(
            &mut text,
            &format!(
                "{} found · {} skipped · {}",
                locale.format_u64(self.progress.discovered),
                locale.format_u64(self.progress.skipped),
                counted(self.progress.errors, "error", "errors", locale)
            ),
        );
        if let Some(summary) = self.run.as_ref().map(|run| &run.summary) {
            for diagnostic in &summary.diagnostics {
                section(&mut text, &describe(diagnostic));
            }
        }
        text
    }
}
