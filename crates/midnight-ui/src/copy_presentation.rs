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
        state
            .borrow_mut()
            .confirm_copy_dialog(token.as_str(), destination.as_str(), send.as_ref());
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
    window.set_copy_draft_body(format!("{}\n\nFrom\n{}", draft.summary, draft.source).into());
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
    let describe = |item: &MutationItem| {
        let size = item.metadata.size.map_or_else(
            || "Unknown size".into(),
            |size| format!("{} bytes", presenter.number_locale.format_u64(size)),
        );
        let modified = time_presentation::modified(&item.metadata).map_or_else(
            || "Unknown modified time".into(),
            |time| format!("{} {}", time.date, time.time),
        );
        format!("{}\n{size} · {modified}", item.path.display())
    };
    window.set_copy_body(format!("Source\n{}\n\nExisting destination\n{}\n\nChoose another name or skip. Replacement is unavailable.",
        describe(&conflict.source), describe(&conflict.target)).into());
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
