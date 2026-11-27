//! Presentation of verified transfers and versioned conflict/error decisions.
use super::*;

fn refresh_draft(window: &MainWindow, presenter: &Presenter) {
    let Some(draft) = presenter
        .copies
        .draft
        .as_ref()
        .filter(|draft| !draft.retire_requested)
    else {
        if !window.get_copy_draft_token().is_empty() {
            // The native host restores its captured invoking item on explicit
            // close. Background retirement must not change owner focus.
            window.set_copy_draft_token("".into());
        }
        return;
    };
    window.set_copy_draft_error(draft.error.as_str().into());
    window.set_copy_draft_error_links(presenter.copy_draft_error_about_links());
    if window.get_copy_draft_token() == draft.token {
        return;
    }
    let destination = draft.destination.to_string_lossy();
    let moving = draft.intent == TransferIntent::Move;
    window.set_copy_draft_move(moving);
    window.set_copy_draft_metadata_enabled(draft.metadata_available());
    let policy = match (moving, presenter.transfer_verification()) {
        (false, TransferVerification::FullSha256) => "Verification: full SHA-256.",
        (false, TransferVerification::SizeAndSamples) => {
            "Quick verification: size + first/middle/last 64 KiB; bytes between samples are not checked."
        }
        (true, TransferVerification::FullSha256) => {
            "Same-volume items are renamed without reading file contents.\nIf copying is needed: full SHA-256 verification before each source is removed."
        }
        (true, TransferVerification::SizeAndSamples) => {
            "Same-volume items are renamed without reading file contents.\nIf copying is needed: size + first/middle/last 64 KiB verification before each source is removed; bytes between samples are not checked."
        }
    };
    // What the job verifies: the Copy or Move button's tooltip.
    window.set_copy_draft_policy(policy.into());
    // The dialog's first line names what is copied and ends where the destination field
    // takes over; where the items come from is its tooltip.
    window.set_copy_draft_body(
        draft
            .items
            .summary(draft.intent, &presenter.number_locale)
            .into(),
    );
    window.set_copy_draft_source(shown_text(&draft.source));
    // The destination field edits the raw path that the dialog confirms.
    window.set_copy_draft_destination(destination.as_ref().into());
    window.set_copy_draft_selection_end(destination.len().min(i32::MAX as usize) as i32);
    window.set_copy_draft_token(draft.token.as_str().into());
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    refresh_draft(window, presenter);
    if let Some((id, version)) = presenter.active_rename_decision() {
        if presenter.operations.preview.is_none()
            && presenter
                .copies
                .draft
                .as_ref()
                .is_none_or(|draft| draft.retire_requested)
            && let Some(conflict) = presenter
                .operations
                .rename_conflicts
                .iter()
                .find(|c| c.id == id && c.version == version)
        {
            refresh_conflict(window, presenter, conflict, "rename");
        }
        return;
    }
    if let Some(failure) = presenter.active_delete_failure() {
        if presenter.operations.preview.is_none()
            && presenter
                .copies
                .draft
                .as_ref()
                .is_none_or(|draft| draft.retire_requested)
        {
            refresh_delete_failure(window, failure);
        }
        return;
    }
    let Some((id, version)) = presenter.active_copy_decision() else {
        if !window.get_copy_token().is_empty() {
            window.set_copy_token("".into());
            window.invoke_close_copy_conflict();
        }
        return;
    };
    if presenter.operations.preview.is_some()
        || presenter
            .copies
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.retire_requested)
    {
        return;
    }
    if let Some(failure) = presenter
        .copies
        .failures
        .iter()
        .find(|failure| failure.id == id && failure.version == version)
    {
        // The job's own word for what it does to the item: copied or moved.
        let intent = presenter
            .copies
            .progress
            .iter()
            .find(|progress| progress.id == failure.id)
            .map(|progress| progress.intent);
        refresh_failure(window, failure, intent);
        return;
    }
    let Some(conflict) = presenter
        .copies
        .conflicts
        .iter()
        .find(|conflict| conflict.id == id && conflict.version == version)
    else {
        return;
    };
    refresh_conflict(window, presenter, conflict, "copy");
}

fn refresh_conflict(
    window: &MainWindow,
    presenter: &Presenter,
    conflict: &CopyConflict,
    prefix: &str,
) {
    let token: SharedString = format!("{prefix}:{}:{}", conflict.id.0, conflict.version).into();
    if window.get_copy_token() == token {
        return;
    }
    let policy = if conflict.can_replace {
        "Replacement keeps the original in a private recovery folder. Recent Jobs show its location; copy details before closing the app."
    } else if conflict.can_merge {
        "Merging keeps existing children. Conflicting files need a separate decision."
    } else {
        "Replacement and merging are unavailable for these items. Choose another name or skip."
    };
    window.set_copy_is_failure(false);
    window.set_copy_can_replace(conflict.can_replace);
    window.set_copy_can_merge(conflict.can_merge);
    window.set_copy_source_details(shown_text(&describe(
        &conflict.source,
        &presenter.number_locale,
    )));
    window.set_copy_target_details(shown_text(&describe(
        &conflict.target,
        &presenter.number_locale,
    )));
    window.set_copy_body(policy.into());
    // The alternate name field edits a raw name.
    window.set_copy_name(
        conflict
            .destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    window.set_copy_token(token);
    window.invoke_show_copy_conflict();
}

fn refresh_failure(window: &MainWindow, failure: &CopyFailure, intent: Option<TransferIntent>) {
    let token: SharedString = format!("copy-error:{}:{}", failure.id.0, failure.version).into();
    if window.get_copy_token() == token {
        return;
    }
    window.set_copy_is_failure(true);
    window.set_copy_can_retry(failure.can_retry);
    window.set_copy_can_skip(failure.can_skip);
    window.set_copy_can_replace(false);
    window.set_copy_can_merge(false);
    window.set_copy_source_details("".into());
    window.set_copy_target_details("".into());
    window.set_copy_body(shown_text(&format_failure(
        failure,
        intent,
        append_split_path,
    )));
    window.set_copy_name("".into());
    window.set_copy_token(token);
    window.invoke_show_copy_conflict();
}

/// A Delete failure in the shared failure prompt (`delete-error:{job}:{version}`), a
/// Trash refusal among them: the item is kept, and Retry, Skip or Cancel decide.
fn refresh_delete_failure(window: &MainWindow, failure: &DeleteFailure) {
    let token: SharedString = format!("delete-error:{}:{}", failure.id.0, failure.version).into();
    if window.get_copy_token() == token {
        return;
    }
    window.set_copy_is_failure(true);
    window.set_copy_can_retry(failure.can_retry);
    window.set_copy_can_skip(failure.can_skip);
    window.set_copy_can_replace(false);
    window.set_copy_can_merge(false);
    window.set_copy_source_details("".into());
    window.set_copy_target_details("".into());
    window.set_copy_body(shown_text(&delete_failure_details(failure)));
    window.set_copy_name("".into());
    window.set_copy_token(token);
    window.invoke_show_copy_conflict();
}

/// What the Delete was doing, why it stopped, and the item (or the folder that keeps
/// its items) split into folder and name.
pub(crate) fn delete_failure_details(failure: &DeleteFailure) -> String {
    let action = match failure.mode {
        DeleteMode::Trash => "Moving to Trash",
        DeleteMode::Permanent => "Deleting permanently",
    };
    let mut detail = format!("{action}\n\n{}", failure.error.message);
    // A refusal by the Trash after the Delete started (its setting changed since F8, or
    // the item is too large for it): the item stays, as for any other failure.
    if failure.error.is_trash_unavailable() {
        detail.push_str(
            "\n\nThe Trash did not take this item, and it was kept. Retry once the Trash \
             takes it, Skip to keep it, or use Shift+Delete to delete it permanently.",
        );
    }
    append_split_path(&mut detail, "Item", &failure.error.path);
    detail
}

pub(crate) fn failure_details(failure: &CopyFailure) -> String {
    format_failure(failure, None, |detail, heading, path| {
        detail.push_str(&format!("\n\n{heading}\n{}", path.display()));
    })
}

/// `intent` names what the job does to the item; without it the wording is neutral.
fn format_failure(
    failure: &CopyFailure,
    intent: Option<TransferIntent>,
    append_path: impl Fn(&mut String, &str, &std::path::Path),
) -> String {
    let stage = match failure.stage {
        CopyErrorStage::Preparing => "Preparing the transfer",
        CopyErrorStage::Reading => "Reading the source",
        CopyErrorStage::Writing => "Writing the destination",
        CopyErrorStage::Syncing => "Saving the destination",
        CopyErrorStage::Verifying => "Verifying the destination",
        CopyErrorStage::Publishing => match intent {
            Some(TransferIntent::Copy) => "Putting the copied item in place",
            Some(TransferIntent::Move) => "Putting the moved item in place",
            None => "Putting the verified item in place",
        },
        CopyErrorStage::Enumerating => "Reading folder contents",
        CopyErrorStage::CreatingDirectory => "Creating a destination folder",
        CopyErrorStage::MovingSource => "Moving the source",
        CopyErrorStage::RemovingSource => "Removing the verified source",
        CopyErrorStage::Cleanup => "Cleaning up temporary files",
        CopyErrorStage::PreservingMetadata => {
            "Preserving times and attributes (the item itself was copied)"
        }
    };
    let mut detail = format!("{stage}\n\n{}", failure.error.message);
    if let Some(source) = &failure.source {
        append_path(&mut detail, "Source", source);
    }
    append_path(&mut detail, "Destination", &failure.destination);
    if !failure.error.path.as_os_str().is_empty()
        && failure.error.path != failure.destination
        && failure.source.as_ref() != Some(&failure.error.path)
    {
        append_path(&mut detail, "Reported path", &failure.error.path);
    }
    detail.push_str(&format!("\n\nOperation: {}", failure.error.operation));
    detail
}

pub(super) fn path_parts(path: &std::path::Path) -> (SharedString, SharedString) {
    let Some(name) = path.file_name() else {
        return (
            path.to_string_lossy().as_ref().into(),
            SharedString::default(),
        );
    };
    let directory = path
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .to_string_lossy();
    (
        directory.as_ref().into(),
        name.to_string_lossy().as_ref().into(),
    )
}

fn append_split_path(detail: &mut String, heading: &str, path: &std::path::Path) {
    let (directory, name) = path_parts(path);
    detail.push_str(&format!("\n\n{heading}"));
    if !directory.is_empty() {
        detail.push_str(&format!("\n{directory}"));
    }
    if !name.is_empty() {
        detail.push_str(&format!("\n{name}"));
    }
}

fn describe(item: &MutationItem, locale: &NumberLocale) -> String {
    let size = item
        .metadata
        .size
        .filter(|_| item.kind == EntryKind::File)
        .map_or_else(
            || "Unknown size".into(),
            |size| {
                format!(
                    "{} ({})",
                    crate::job_presentation::counted(size, "byte", "bytes", locale),
                    format_size(size, locale)
                )
            },
        );
    let modified = time_presentation::modified(&item.metadata).map_or_else(
        || "Unknown modified time".into(),
        |time| format!("{} {}", time.date, time.time),
    );
    let kind = match item.kind {
        EntryKind::File => "File",
        EntryKind::Directory => "Folder",
        EntryKind::DirectoryLink | EntryKind::Symlink => "Link",
        EntryKind::Other => "Special item",
    };
    let precise = item
        .metadata
        .modified
        .and_then(time_presentation::utc_precise)
        .map_or_else(String::new, |utc| format!("\n{utc}"));
    format!(
        "{}\n\n{kind}\n{size}\nModified: {modified}{precise}",
        item.path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_details_keep_full_unicode_paths_separate_from_the_message() {
        let source = format!("資料/{}/source.txt", "long folder/".repeat(70));
        let destination = format!("target/{}/資料.txt", "long folder/".repeat(70));
        let failure = CopyFailure {
            id: OperationId(1),
            version: 7,
            error: FsError::from_io(
                "write",
                std::path::Path::new("private/staging"),
                std::io::Error::other("Access denied"),
            ),
            source: Some(source.clone().into()),
            destination: destination.clone().into(),
            stage: CopyErrorStage::Writing,
            can_retry: true,
            can_skip: true,
        };
        let detail = failure_details(&failure);
        assert!(detail.contains(&format!("\n\nSource\n{source}")));
        assert!(detail.contains(&format!("\n\nDestination\n{destination}")));
        assert!(detail.contains("\n\nReported path\nprivate/staging"));
        assert!(detail.contains("\n\nAccess denied\n\n"));
        let display = format_failure(&failure, None, append_split_path);
        let source_directory = std::path::Path::new(&source).parent().unwrap();
        let target_directory = std::path::Path::new(&destination).parent().unwrap();
        assert!(display.contains(&format!(
            "Source\n{}\nsource.txt",
            source_directory.display()
        )));
        assert!(display.contains(&format!(
            "Destination\n{}\n資料.txt",
            target_directory.display()
        )));
    }
    #[test]
    fn a_refused_move_names_the_move_phase_and_operation() {
        let failure = CopyFailure {
            id: OperationId(1),
            version: 1,
            error: FsError::from_io(
                "move",
                std::path::Path::new("data/locked"),
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Permission denied: the folder is read-only.",
                ),
            ),
            source: Some("data/locked".into()),
            destination: "target".into(),
            stage: CopyErrorStage::MovingSource,
            can_retry: true,
            can_skip: true,
        };
        let display = format_failure(&failure, Some(TransferIntent::Move), append_split_path);
        assert!(
            display.starts_with("Moving the source\n\nPermission denied: the folder is read-only."),
            "{display}"
        );
        assert!(display.ends_with("\n\nOperation: move"), "{display}");
    }
    #[test]
    fn a_failure_putting_the_item_in_place_says_copied_or_moved_never_published() {
        let failure = CopyFailure {
            id: OperationId(1),
            version: 1,
            error: FsError::from_io(
                "rename",
                std::path::Path::new("target/report.txt"),
                std::io::Error::other("The name exists."),
            ),
            source: Some("data/report.txt".into()),
            destination: "target/report.txt".into(),
            stage: CopyErrorStage::Publishing,
            can_retry: true,
            can_skip: true,
        };
        for (intent, stage) in [
            (
                Some(TransferIntent::Copy),
                "Putting the copied item in place",
            ),
            (
                Some(TransferIntent::Move),
                "Putting the moved item in place",
            ),
            // Jobs details, which do not pass the job's kind.
            (None, "Putting the verified item in place"),
        ] {
            let display = format_failure(&failure, intent, append_split_path);
            assert!(display.starts_with(&format!("{stage}\n\n")), "{display}");
            assert!(!display.to_lowercase().contains("publish"), "{display}");
        }
        assert!(!failure_details(&failure).to_lowercase().contains("publish"));
    }
    #[test]
    fn conflict_details_distinguish_types_unknowns_and_explicit_precise_utc() {
        for (kind, label) in [
            (EntryKind::File, "File"),
            (EntryKind::Directory, "Folder"),
            (EntryKind::Symlink, "Link"),
            (EntryKind::Other, "Special item"),
        ] {
            let mut item = MutationItem {
                path: "資料/source".into(),
                kind,
                metadata: EntryMetadata::default(),
            };
            let unknown = describe(&item, &NumberLocale::default());
            assert!(unknown.contains(&format!("\n{label}\nUnknown size\nModified: Unknown")));
            item.metadata.size = Some(123);
            item.metadata.modified =
                Some(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(100));
            item.metadata.localized_modified = Some(LocalizedDateTime {
                date: "local date".into(),
                time: "1 PM".into(),
            });
            let detail = describe(&item, &NumberLocale::default());
            assert!(detail.contains("Modified: local date 1 PM"));
            assert!(detail.contains("1970-01-01 00:00:00.000000100 UTC"));
            assert_eq!(detail.contains("123 bytes"), kind == EntryKind::File);
            assert!(detail.contains("資料"));
        }
    }
    #[test]
    fn a_one_byte_file_is_described_in_the_singular() {
        let item = MutationItem {
            path: "one.txt".into(),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(1),
                ..EntryMetadata::default()
            },
        };
        let detail = describe(&item, &NumberLocale::default());
        assert!(detail.contains("\nFile\n1 byte (1 B)\n"), "{detail}");
    }
}
