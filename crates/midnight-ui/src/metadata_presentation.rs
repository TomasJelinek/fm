//! Bounded metadata forms and conflict presentation; native work is sent to the engine.
use super::*;

pub(super) fn bind(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) {
    macro_rules! callback {
        ($name:ident, |$p:ident, $s:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let send = send.clone(); let weak = window.as_weak();
            window.$name(move |$($arg),*| {
                let Some(window) = weak.upgrade() else { return; };
                { let mut borrow = state.borrow_mut(); let $p = &mut *borrow; let $s = send.as_ref(); $body }
                refresh_properties(&window, &state.borrow(), false);
            });
        }};
    }
    callback!(on_open_metadata, |p, s, id, case| {
        if let Some(pane) = match id {
            0 => Some(PaneId::Left),
            1 => Some(PaneId::Right),
            _ => None,
        } {
            p.open_metadata(pane, case, s);
        }
    });
    callback!(on_dirty_metadata, |p, _s, token| {
        p.dirty_metadata(token.as_str());
    });
    callback!(on_preview_metadata, |p, s, token, draft| {
        p.preview_metadata(token.as_str(), metadata_input::patch(&draft), s);
    });
    callback!(on_confirm_metadata, |p, s, token, revision, policy| {
        p.confirm_metadata(token.as_str(), revision.as_str(), policy, s);
    });
    callback!(on_cancel_metadata, |p, s, token| {
        p.cancel_metadata(token.as_str(), s);
    });
    callback!(on_decide_metadata_conflict, |p, s, token, choice| {
        p.decide_metadata_conflict(token.as_str(), choice, s);
    });
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.metadata;
    window.set_metadata_start_case(state.start_case);
    window.set_metadata_revision(state.revision.to_string().into());
    window.set_metadata_error(state.error.as_str().into());
    window.set_metadata_busy(state.busy);
    window.set_metadata_ready(state.can_apply());
    let caps = state
        .preview
        .as_ref()
        .map(|p| p.capabilities)
        .unwrap_or_default();
    window.set_metadata_can_windows(caps.windows_attributes != 0);
    window.set_metadata_windows_mask(caps.windows_attributes as i32);
    window.set_metadata_can_unix(caps.unix_permissions != 0);
    window.set_metadata_can_compress(caps.compressed);
    window.set_metadata_can_created(caps.created);
    window.set_metadata_can_modified(caps.modified);
    window.set_metadata_can_accessed(caps.accessed);
    window.set_metadata_can_rename(caps.rename);
    if let Some(preview) = &state.preview {
        let suffix = if preview.hard_linked > 0 {
            " · Metadata changes also affect other hard links to these files."
        } else {
            ""
        };
        window.set_metadata_summary(
            format!(
                "{} selected items · {} name conflicts · {} unsupported{}",
                preview.total, preview.conflicts, preview.unsupported, suffix
            )
            .into(),
        );
        window.set_metadata_preview_text(preview_text(preview).into());
    } else {
        window.set_metadata_summary("Preparing captured selection…".into());
        window.set_metadata_preview_text("".into());
    }
    window.set_metadata_token(
        state
            .id
            .map(|id| id.0.to_string())
            .unwrap_or_default()
            .into(),
    );
    if let Some(conflict) = presenter.operations.metadata_conflicts.first() {
        show_conflict(window, conflict, false);
    } else {
        window.set_metadata_conflict_token("".into());
        window.invoke_close_metadata_conflict();
    }
}

pub(super) fn show_conflict(window: &MainWindow, conflict: &MetadataConflict, force: bool) {
    let token = format!("metadata:{}:{}", conflict.id.0, conflict.token);
    if !force && window.get_metadata_conflict_token() == token {
        return;
    }
    window.set_metadata_conflict_body(format!(
        "Source: {}\nRequested name: {}\n\nThe destination already exists. No existing entry will be overwritten. Skipping leaves this item's remaining changes unapplied; completed changes elsewhere are retained.",
        conflict.source.display(), conflict.destination.display()).into());
    window.set_metadata_conflict_token(token.into());
    window.invoke_show_metadata_conflict();
}

fn time(time: std::time::SystemTime) -> String {
    time_presentation::utc_fallback(time)
        .map(|value| format!("{} {}", value.date, value.time))
        .unwrap_or_else(|| "outside the display calendar".into())
}

fn requested(patch: &MetadataPatch) -> String {
    let mut changes = Vec::new();
    for (flag, name) in [
        (EDIT_READ_ONLY, "Read-only"),
        (EDIT_HIDDEN, "Hidden"),
        (EDIT_SYSTEM, "System"),
        (EDIT_ARCHIVE, "Archive"),
    ] {
        if patch.windows_mask & flag != 0 {
            changes.push(format!(
                "{} {name}",
                if patch.windows_value & flag != 0 {
                    "Set"
                } else {
                    "Clear"
                }
            ));
        }
    }
    if patch.unix_mask != 0 {
        changes.push(format!(
            "Permissions {:04o} (edited mask {:04o})",
            patch.unix_value, patch.unix_mask
        ));
    }
    if let Some(compressed) = patch.compressed {
        changes.push(if compressed { "Compress" } else { "Uncompress" }.into());
    }
    for (name, value) in [
        ("Created", patch.created),
        ("Modified", patch.modified),
        ("Accessed", patch.accessed),
    ] {
        if let TimeEdit::Set(value) = value {
            changes.push(format!("{name}: {}", time(value)));
        }
    }
    if let Some(case) = patch.case {
        changes.push(format!("Filename case: {:?}, {:?}", case.mode, case.scope));
    }
    if changes.is_empty() {
        changes.push("No changes requested. Current values are shown below.".into());
    }
    if patch.recursive {
        changes.push("Includes descendants; children are processed before their parent.".into());
    }
    changes.join("\n")
}

fn preview_text(preview: &MetadataPreview) -> String {
    let mut text = requested(&preview.patch);
    for row in &preview.sample {
        text.push_str(&format!("\n\n{}", row.path.display()));
        if let Some(name) = &row.proposed_name {
            text.push_str(&format!("\n→ {}", name.to_string_lossy()));
        }
        let current = &row.current;
        if let Some(mode) = current.unix_mode {
            text.push_str(&format!("\nCurrent permissions: {:04o}", mode & 0o7777));
        }
        if let Some(attributes) = current.windows_attributes {
            let flags = [
                (EDIT_READ_ONLY, "Read-only"),
                (EDIT_HIDDEN, "Hidden"),
                (EDIT_SYSTEM, "System"),
                (EDIT_ARCHIVE, "Archive"),
            ]
            .into_iter()
            .filter_map(|(bit, label)| (attributes & bit != 0).then_some(label))
            .collect::<Vec<_>>();
            text.push_str(&format!(
                "\nCurrent attributes: {}",
                if flags.is_empty() {
                    "None".into()
                } else {
                    flags.join(", ")
                }
            ));
        }
        for (label, flag) in [
            ("Compressed", current.compressed),
            ("Encrypted", current.encrypted),
        ] {
            if let Some(flag) = flag {
                text.push_str(&format!(" · {label}: {}", if flag { "Yes" } else { "No" }));
            }
        }
        for (label, value) in [
            ("Created", current.created),
            ("Modified", current.modified),
            ("Accessed", current.accessed),
        ] {
            if let Some(value) = value {
                text.push_str(&format!("\n{label}: {}", time(value)));
            }
        }
        if let Some(issue) = &row.issue {
            text.push_str(&format!("\n{issue}"));
        }
    }
    if preview.total > preview.sample.len() as u64 {
        text.push_str("\n\nPreview shows the first 32 selected roots; the complete captured selection is included.");
    }
    text
}

pub(super) fn result_detail(result: &MetadataResult) -> String {
    let mut text = format!(
        "\n{} examined · {} changed · {} skipped · {} failed",
        result.processed, result.changed, result.skipped, result.failed
    );
    if !result.sample.is_empty() {
        text.push_str("\nOutcomes (paths at the time of each edit):");
    }
    for item in &result.sample {
        let changes = item.changes;
        let fields = [
            (changes.renamed, "name"),
            (changes.attributes, "attributes"),
            (changes.permissions, "permissions"),
            (changes.compressed, "compression"),
            (changes.created, "created time"),
            (changes.modified, "modified time"),
            (changes.accessed, "accessed time"),
        ]
        .into_iter()
        .filter_map(|(changed, field)| changed.then_some(field))
        .collect::<Vec<_>>();
        text.push_str(&format!(
            "\n{}: {}",
            item.current_path.display(),
            if fields.is_empty() {
                "unchanged".into()
            } else {
                fields.join(", ")
            }
        ));
        if let Some(error) = &item.error {
            text.push_str(&format!("; {error}"));
        }
    }
    if result.processed > result.sample.len() as u64 {
        text.push_str("\nAdditional outcomes are included in the totals.");
    }
    text
}
