//! Presentation of verified copy counters and versioned name conflicts.
use super::*;

pub(super) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let state = presenter.clone();
    window.on_confirm_copy_draft(move |token, destination| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if window.get_copy_draft_preserve_time() {
            state.borrow_mut().confirm_copy_dialog_metadata(
                token.as_str(),
                destination.as_str(),
                TransferMetadataPolicy::RequireModifiedTime,
                send.as_ref(),
            );
        } else {
            state.borrow_mut().confirm_copy_dialog(
                token.as_str(),
                destination.as_str(),
                send.as_ref(),
            );
        }
        // Showing/closing popups synchronously reads presenter-backed row models.
        crate::refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    window.on_cancel_copy_draft(move |token| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        presenter.borrow_mut().cancel_copy_dialog(token.as_str());
        crate::refresh(&window, &presenter.borrow());
        window.invoke_focus_files();
    });
}

fn refresh_draft(window: &MainWindow, presenter: &Presenter) {
    let Some(draft) = &presenter.copies.draft else {
        if !window.get_copy_draft_token().is_empty() {
            window.set_copy_draft_token("".into());
            window.invoke_focus_files();
        }
        return;
    };
    window.set_copy_draft_error(draft.error.as_str().into());
    if window.get_copy_draft_token() == draft.token {
        return;
    }
    let destination = draft.destination.to_string_lossy();
    let moving = draft.intent == TransferIntent::RecoverableMove;
    window.set_copy_draft_move(moving);
    window.set_copy_draft_metadata_enabled(draft.metadata_available());
    window.set_copy_draft_preserve_time(false);
    let policy = if moving {
        "Up to 64 regular files; folders and links are unsupported. Every file must be copied and checked before originals move into protected recovery folders. Source disk space is not freed.\n\n"
    } else {
        ""
    };
    window.set_copy_draft_body(
        format!("{}{}\n\nFrom\n{}", policy, draft.summary, draft.source).into(),
    );
    window.set_copy_draft_destination(destination.as_ref().into());
    window.set_copy_draft_selection_end(destination.len().min(i32::MAX as usize) as i32);
    window.set_copy_draft_token(draft.token.as_str().into());
    window.invoke_show_copy_draft();
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    refresh_draft(window, presenter);
    if let Some(progress) = presenter.copies.progress.last() {
        let token: SharedString = progress.id.0.to_string().into();
        if window.get_copy_latest() != token {
            window.set_copy_latest(token);
            if !progress.state.is_terminal()
                && !presenter.operations.closing
                && presenter.operations.preview.is_none()
                && presenter.copies.draft.is_none()
            {
                window.invoke_show_operations();
            }
        }
    }
    let Some(conflict) = presenter.copies.conflicts.first() else {
        if !window.get_copy_token().is_empty() {
            window.set_copy_token("".into());
            window.invoke_close_copy_conflict();
        }
        return;
    };
    if presenter.operations.preview.is_some() || presenter.copies.draft.is_some() {
        return;
    }
    let token: SharedString = format!("copy:{}:{}", conflict.id.0, conflict.version).into();
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
    window.set_copy_can_replace(conflict.can_replace);
    window.set_copy_can_merge(conflict.can_merge);
    window.set_copy_source_details(describe(&conflict.source, &presenter.number_locale).into());
    window.set_copy_target_details(describe(&conflict.target, &presenter.number_locale).into());
    window.set_copy_body(policy.into());
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
