//! How a finished job ended, for its window: what it did and did not do, and the
//! errors it reported. Informational notes and the work a cancellation left undone
//! are counted, never listed; only errors are listed, one row each.
use crate::{JobIssue, error_presentation::path_text, path_tail, shown_text};
use midnight_core::{
    CopyOperationState, CopyProgress, DeleteMode, DiscoveryScope, DiscoveryState, FsError,
    MoveSourceResidual, NumberLocale, OperationProgress, OperationState, TransferIntent,
};
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;

#[cfg(test)]
#[path = "job_outcome_tests.rs"]
mod tests;

/// The errors a job's window lists, and their count on one line.
pub(super) struct Issues {
    pub(super) rows: ModelRc<JobIssue>,
    pub(super) summary: SharedString,
}

/// What a finished Copy or Move did and did not do. Empty while it runs and after a
/// clean completion, which needs nothing reviewed.
pub(super) fn transfer(p: &CopyProgress, failed: bool, locale: &NumberLocale) -> String {
    use CopyOperationState as S;
    if !p.state.is_terminal()
        || (p.state == S::Completed
            && !failed
            && p.closure.is_none()
            && p.skipped == 0
            && p.metadata_not_preserved == 0)
    {
        return String::new();
    }
    let moving = p.intent == TransferIntent::Move;
    let verb = if moving { "moved" } else { "copied" };
    let n = |value: u64| locale.format_u64(value);
    let mut lines = Vec::new();
    if p.state == S::Cancelled {
        let started = p.files_completed > 0
            || p.directories_created > 0
            || p.native_entries_moved > 0
            || p.completed_roots > 0;
        lines.push(
            match (moving, started) {
                (_, true) => cancelled_after_start(p, moving),
                (false, false) => "Copy cancelled; nothing was copied.",
                (true, false) => "Move cancelled; nothing was moved.",
            }
            .to_owned(),
        );
    }
    let totals = lines.len();
    if p.native_entries_moved > 0 {
        lines.push(format!("Moved by renaming: {}", n(p.native_entries_moved)));
    }
    let uncommitted = p.closure.and_then(|closure| closure.uncommitted_files);
    // A file is moved once its source is gone too. Renamed entries are counted
    // above, so with them the number of files left behind is unknown.
    let (done, not_done) = if moving {
        let left = uncommitted
            .filter(|_| p.native_entries_moved == 0)
            .map(|files| {
                files.saturating_add(p.files_completed.saturating_sub(p.source_files_removed))
            });
        (p.source_files_removed, left)
    } else {
        (p.files_completed, uncommitted)
    };
    if done > 0 || not_done.is_some_and(|files| files > 0) {
        let mut line = format!("Files {verb}: {}", n(done));
        if let Some(files) = not_done.filter(|files| *files > 0) {
            line.push_str(&format!(" · Not {verb}: {}", n(files)));
        }
        lines.push(line);
    }
    // Bytes count publications; for a Move they are moved bytes only while every
    // published file also had its source removed.
    let bytes_moved = p.native_entries_moved == 0 && p.files_completed == p.source_files_removed;
    if let Some(bytes) = p
        .closure
        .and_then(|closure| closure.uncommitted_bytes)
        .filter(|bytes| *bytes > 0 && (!moving || bytes_moved))
    {
        lines.push(format!(
            "Bytes {verb}: {} · Not {verb}: {}",
            n(p.completed_bytes),
            n(bytes)
        ));
    }
    let unfinished_roots = p.closure.map_or(0, |closure| closure.unfinished_roots);
    if lines.len() == totals || (not_done.is_none() && unfinished_roots > 0) {
        let mut line = format!("Selected items {verb}: {}", n(p.completed_roots));
        if unfinished_roots > 0 {
            line.push_str(&format!(" · Not {verb}: {}", n(unfinished_roots)));
        }
        lines.push(line);
    }
    if p.skipped > 0 {
        lines.push(if p.skipped == 1 {
            format!("1 skipped item was not {verb}.")
        } else {
            format!("{} skipped items were not {verb}.", n(p.skipped))
        });
    }
    // A link left out where no links are created has its own note in Copy details.
    if p.links_left_out > 0 {
        lines.push(if p.links_left_out == 1 {
            "1 link was left out: no links are created there; Copy details names it.".to_owned()
        } else {
            format!(
                "{} links were left out: no links are created there; Copy details names each.",
                n(p.links_left_out)
            )
        });
    }
    // Each item copied without all of its times or attributes has a note in Copy details.
    if p.metadata_not_preserved > 0 {
        lines.push(if p.metadata_not_preserved == 1 {
            "1 item was copied without all of its times or attributes; Copy details names it."
                .to_owned()
        } else {
            format!(
                "{} items were copied without all of their times or attributes; Copy details names each.",
                n(p.metadata_not_preserved)
            )
        });
    }
    // Copy records a note otherwise only for a link it could not follow and so copied
    // as a link; Copy details lists each one with its reason.
    let link_notes = p
        .note_count
        .saturating_sub(p.links_left_out)
        .saturating_sub(p.metadata_not_preserved);
    if link_notes > 0 {
        lines.push(if link_notes == 1 {
            "1 link was copied as a link; Copy details says why.".to_owned()
        } else {
            format!(
                "{} links were copied as links; Copy details says why.",
                n(link_notes)
            )
        });
    }
    if p.recovery_count > 0 {
        lines.push(if p.recovery_count == 1 {
            "1 file was kept in a recovery folder; Copy details shows where.".to_owned()
        } else {
            format!(
                "{} files were kept in recovery folders; Copy details shows where.",
                n(p.recovery_count)
            )
        });
    }
    if p.source_delete_pending > 0 {
        lines.push(format!(
            "Source removals accepted by Windows: {}. Programs that still have them open can delay freeing their space.",
            n(p.source_delete_pending)
        ));
    }
    if p.source_removal_durability_unconfirmed > 0 {
        lines.push(format!(
            "Source changes the system could not confirm as saved to disk: {}",
            n(p.source_removal_durability_unconfirmed)
        ));
    }
    lines.join("\n")
}

/// What became of a Copy or Move cancelled after it started, in the engine's words
/// from the job's message, which its details show, so that the two never disagree.
/// A Move can leave files copied to the destination with their sources restored.
fn cancelled_after_start(p: &CopyProgress, moving: bool) -> &str {
    let (start, said) = if moving {
        (
            "Move cancelled;",
            "Move cancelled; completed moves remain, and files already copied to the destination are kept.",
        )
    } else {
        (
            "Copy cancelled;",
            "Copy cancelled; files already copied are kept.",
        )
    };
    p.message
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(start))
        .unwrap_or(said)
}

/// What a finished Delete did and did not do. Empty while it runs and after a clean
/// completion.
pub(super) fn deletion(p: &OperationProgress, mode: DeleteMode, locale: &NumberLocale) -> String {
    if !p.state.is_terminal() {
        return String::new();
    }
    if p.state == OperationState::Completed && p.failed == 0 && p.errors.is_empty() {
        return String::new();
    }
    let trash = mode == DeleteMode::Trash;
    let (verb, not) = if trash {
        ("moved to the Trash", "Not moved")
    } else {
        ("deleted", "Not deleted")
    };
    let n = |value: u64| locale.format_u64(value);
    let mut lines = Vec::new();
    if p.state == OperationState::Cancelled {
        let started = p.completed > 0 || p.processed_entries > 0;
        lines.push(
            match (trash, started) {
                (false, true) => "Delete cancelled; items already deleted are not restored.",
                (false, false) => "Delete cancelled; nothing was deleted.",
                (true, true) => "Move to Trash cancelled; items already in the Trash stay there.",
                (true, false) => "Move to Trash cancelled; nothing was moved to the Trash.",
            }
            .to_owned(),
        );
    }
    let totals = lines.len();
    let discovery = &p.discovery;
    if discovery.scope == DiscoveryScope::Recursive {
        let mut line = format!("Items {verb}: {}", n(p.processed_entries));
        if let Some(rest) = discovery
            .entries()
            .filter(|_| discovery.state == DiscoveryState::Complete)
            .and_then(|entries| entries.checked_sub(p.processed_entries))
            .filter(|rest| *rest > 0)
        {
            line.push_str(&format!(" · {not}: {}", n(rest)));
        }
        lines.push(line);
    }
    let rest = p.total.saturating_sub(p.completed);
    if lines.len() == totals || rest > 0 {
        let mut line = format!("Selected items {verb}: {}", n(p.completed));
        if rest > 0 {
            line.push_str(&format!(" · {not}: {}", n(rest)));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// A finished Rename, creation or metadata job that did not complete cleanly: the
/// lines of its message that its listed errors do not already say.
pub(super) fn change(p: &OperationProgress) -> String {
    if !p.state.is_terminal() || (p.state == OperationState::Completed && p.failed == 0) {
        return String::new();
    }
    let errors: Vec<String> = p.errors.iter().map(ToString::to_string).collect();
    p.message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !errors.iter().any(|error| error == line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The errors a Copy or Move reported, then each source a Move kept in a temporary
/// location.
pub(super) fn transfer_issues(p: &CopyProgress, locale: &NumberLocale) -> Issues {
    let rows = p
        .errors
        .iter()
        .map(error_row)
        .chain(p.source_residuals.iter().map(residual_row))
        .collect();
    let summary = [
        counted_list(p.failed, p.errors.len(), "error", "errors", locale),
        counted_list(
            p.source_residual_count,
            p.source_residuals.len(),
            "source kept in a temporary location",
            "sources kept in temporary locations",
            locale,
        ),
    ];
    issues(rows, &summary)
}

/// The errors a Delete, Rename, creation or metadata job reported.
pub(super) fn mutation_issues(p: &OperationProgress, locale: &NumberLocale) -> Issues {
    let rows = p.errors.iter().map(error_row).collect();
    issues(
        rows,
        &[counted_list(
            p.failed,
            p.errors.len(),
            "error",
            "errors",
            locale,
        )],
    )
}

fn issues(rows: Vec<JobIssue>, summary: &[Option<String>]) -> Issues {
    Issues {
        rows: if rows.is_empty() {
            ModelRc::default()
        } else {
            ModelRc::from(Rc::new(VecModel::from(rows)))
        },
        summary: summary
            .iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" · ")
            .into(),
    }
}

/// "3 errors", or "20 errors, 16 listed" when the job kept details of only some.
fn counted_list(
    count: u64,
    listed: usize,
    singular: &str,
    plural: &str,
    locale: &NumberLocale,
) -> Option<String> {
    let listed = listed as u64;
    let count = count.max(listed);
    if count == 0 {
        return None;
    }
    let noun = if count == 1 { singular } else { plural };
    let mut text = format!("{} {noun}", locale.format_u64(count));
    if listed == 0 {
        text.push_str("; their details were not kept");
    } else if listed < count {
        text.push_str(&format!(", {} listed", locale.format_u64(listed)));
    }
    Some(text)
}

/// Display text only: a path keeps its identity escaped (see `path_text`) and shows
/// bidirectional controls as escapes, as everywhere in the window.
fn error_row(error: &FsError) -> JobIssue {
    let path = shown_text(&path_text(&error.path));
    let message = error.message.trim();
    let title = message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(error.operation);
    let detail = if message.is_empty() {
        format!("Operation: {}", error.operation)
    } else {
        format!("{message}\n\nOperation: {}", error.operation)
    };
    JobIssue {
        title: shown_text(title),
        path_tail: path_tail(&path),
        path,
        detail: shown_text(&detail),
    }
}

fn residual_row(residual: &MoveSourceResidual) -> JobIssue {
    let path = shown_text(&path_text(&residual.temporary.path));
    let title = if residual.matched_source {
        "Restoring the source failed; the original is kept here"
    } else {
        "Cleaning up failed; check this temporary location"
    };
    JobIssue {
        title: title.into(),
        path_tail: path_tail(&path),
        path,
        detail: shown_text(&format!("Original source\n{}", path_text(&residual.source))),
    }
}
