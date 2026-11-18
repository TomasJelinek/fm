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
    window.set_copy_draft_policy(policy.into());
    window.set_copy_draft_body(shown_text(&format!(
        "{}\n\nFrom\n{}",
        draft.summary, draft.source
    )));
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
        refresh_failure(window, failure);
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

fn refresh_failure(window: &MainWindow, failure: &CopyFailure) {
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
    window.set_copy_body(shown_text(&failure_details(failure)));
    window.set_copy_name("".into());
    window.set_copy_token(token);
    window.invoke_show_copy_conflict();
}

pub(crate) fn failure_details(failure: &CopyFailure) -> String {
    let stage = match failure.stage {
        CopyErrorStage::Preparing => "Preparing the transfer",
        CopyErrorStage::Reading => "Reading the source",
        CopyErrorStage::Writing => "Writing the destination",
        CopyErrorStage::Syncing => "Saving the destination",
        CopyErrorStage::Verifying => "Verifying the destination",
        CopyErrorStage::Publishing => "Publishing the verified file",
        CopyErrorStage::Enumerating => "Reading folder contents",
        CopyErrorStage::CreatingDirectory => "Creating a destination folder",
        CopyErrorStage::RemovingSource => "Removing the verified source",
        CopyErrorStage::Cleanup => "Cleaning up temporary files",
    };
    let mut detail = format!("{stage}\n\n{}", failure.error.message);
    if let Some(source) = &failure.source {
        detail.push_str(&format!("\n\nSource\n{}", source.display()));
    }
    detail.push_str(&format!(
        "\n\nDestination\n{}",
        failure.destination.display()
    ));
    if !failure.error.path.as_os_str().is_empty()
        && failure.error.path != failure.destination
        && failure.source.as_ref() != Some(&failure.error.path)
    {
        detail.push_str(&format!(
            "\n\nReported path\n{}",
            failure.error.path.display()
        ));
    }
    detail.push_str(&format!("\n\nOperation: {}", failure.error.operation));
    detail
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
                    "{} bytes ({})",
                    locale.format_u64(size),
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
}
