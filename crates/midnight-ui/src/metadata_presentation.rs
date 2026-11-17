//! Bounded metadata forms and conflict presentation; native work is sent to the engine.
use super::*;
use crate::error_presentation::path_text;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ParentItemTraversalMode,
    items::AccessibleRole, window::WindowInner,
};

pub(super) fn install_permissions(window: &MainWindow) {
    window.global::<UnixPermissionsCodec>().on_parse(|text| {
        match crate::unix_permissions::parse_octal(&text) {
            Ok((mask, value)) => UnixPermissionBits {
                mask: mask as i32,
                value: value as i32,
                valid: true,
            },
            Err(_) => UnixPermissionBits::default(),
        }
    });
    window
        .global::<UnixPermissionsCodec>()
        .on_project(|mode, known, mask, value| {
            let view = crate::unix_permissions::project(mode, known, mask, value);
            UnixPermissionView {
                mode: view.mode as i32,
                known: view.known as i32,
                octal: view.octal.into(),
                symbolic: view.text.into(),
            }
        });
}

/// Close only transient controls owned by this dialog, before replacing fields.
/// Accessible actions can switch topics while a ComboBox popup is still open.
fn dismiss_controls(window: &MainWindow) {
    crate::tooltip::dismiss(window.window());
    let inner = WindowInner::from_pub(window.window());
    let owned = inner
        .active_popups()
        .iter()
        .filter(|popup| {
            let mut ancestor = popup.parent_item.upgrade();
            while let Some(item) = ancestor {
                if item.accessible_role() == AccessibleRole::Groupbox
                    && item
                        .accessible_string_property(AccessibleStringProperty::Label)
                        .is_some_and(|label| label == "Attributes, times and case")
                {
                    return true;
                }
                ancestor = item.parent_item(ParentItemTraversalMode::FindAllParents);
            }
            false
        })
        .map(|popup| popup.popup_id)
        .collect::<Vec<_>>();
    for id in owned.into_iter().rev() {
        inner.close_popup(id);
    }
}

pub(super) fn bind(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) {
    install_permissions(window);
    let weak = window.as_weak();
    window.on_dismiss_metadata_hints(move || {
        if let Some(window) = weak.upgrade() {
            dismiss_controls(&window);
        }
    });
    macro_rules! callback {
        ($name:ident, |$p:ident, $s:ident $(, $arg:ident)*| $body:block) => {{
            callback!($name, true, |$p, $s $(, $arg)*| $body);
        }};
        ($name:ident, $close_children:expr, |$p:ident, $s:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let send = send.clone(); let weak = window.as_weak();
            window.$name(move |$($arg),*| {
                let Some(window) = weak.upgrade() else { return; };
                if $close_children { dismiss_controls(&window); }
                else { crate::tooltip::dismiss(window.window()); }
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
    callback!(on_dirty_metadata, false, |p, _s, token| {
        p.dirty_metadata(token.as_str());
    });
    callback!(on_preview_metadata, |p, s, token, draft| {
        let format = p
            .metadata
            .preview
            .as_ref()
            .and_then(|preview| preview.time_input_format.as_ref());
        let patch = metadata_input::patch(&draft, format);
        p.preview_metadata(token.as_str(), patch, s);
    });
    callback!(on_confirm_metadata, |p, s, token, revision, policy| {
        p.confirm_metadata(token.as_str(), revision.as_str(), policy, s);
    });
    callback!(on_cancel_metadata, |p, s, token| {
        p.cancel_metadata(token.as_str(), s);
    });
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.metadata;
    // The worker reply can replace the editing section while a field hint is
    // open. Retire its transient children before changing the component tree.
    if (!window.get_metadata_preview_current() && state.preview_current())
        || window.get_metadata_token().as_str()
            != state.id.map(|id| id.0.to_string()).unwrap_or_default()
    {
        dismiss_controls(window);
    }
    window.set_metadata_timestamp_example(
        metadata_input::timestamp_example(
            state
                .preview
                .as_ref()
                .and_then(|preview| preview.time_input_format.as_ref()),
        )
        .into(),
    );
    window.set_metadata_start_case(state.start_case);
    window.set_metadata_revision(state.revision.to_string().into());
    window.set_metadata_error(state.error.as_str().into());
    window.set_metadata_busy(state.busy);
    window.set_metadata_ready(state.can_apply());
    window.set_metadata_preview_current(state.preview_current());
    let caps = state
        .preview
        .as_ref()
        .map(|p| p.capabilities)
        .unwrap_or_default();
    window.set_metadata_can_windows(caps.windows_attributes != 0);
    window.set_metadata_windows_mask(caps.windows_attributes as i32);
    window.set_metadata_can_unix(caps.unix_permissions != 0);
    window.set_metadata_unix_supported_mask((caps.unix_permissions & 0o7777) as i32);
    let (known, mode) = state.preview.as_ref().map_or((0, 0), |preview| {
        crate::unix_permissions::common_modes(
            preview.sample.iter().map(|row| row.current.unix_mode),
            preview.total,
        )
    });
    window.set_metadata_unix_current_mode(mode as i32);
    window.set_metadata_unix_known_mask(known as i32);
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
        window.set_metadata_preview_text(preview_status(presenter).into());
        window.set_metadata_list_status(
            format!(
                "{} of {} captured roots shown{}",
                preview.sample.len(),
                preview.total,
                if preview.patch.recursive {
                    " · Descendants are inspected during Apply."
                } else {
                    ""
                }
            )
            .into(),
        );
    } else {
        window.set_metadata_summary("Preparing captured selection…".into());
        window.set_metadata_preview_text("".into());
        window.set_metadata_list_status("Preparing captured files…".into());
    }
    window.set_metadata_token(
        state
            .id
            .map(|id| id.0.to_string())
            .unwrap_or_default()
            .into(),
    );
    // Keep an explicitly resolved job selected while its exact prompt is live.
    // Falling back to the first row on every refresh would retarget its native child.
    let selected = window.get_metadata_conflict_token();
    if let Some(conflict) = presenter
        .operations
        .metadata_conflicts
        .iter()
        .find(|conflict| selected == format!("metadata:{}:{}", conflict.id.0, conflict.token))
        .or_else(|| presenter.operations.metadata_conflicts.first())
    {
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
        path_text(&conflict.source), path_text(&conflict.destination)).into());
    window.set_metadata_conflict_token(token.into());
    window.invoke_show_metadata_conflict();
}

fn time(time: std::time::SystemTime, preview: &MetadataPreview) -> String {
    preview
        .localized_times
        .get(&time)
        .cloned()
        .or_else(|| time_presentation::utc_fallback(time))
        .map(|value| format!("{} {}", value.date, value.time))
        .unwrap_or_else(|| "outside the display calendar".into())
}

fn requested(preview: &MetadataPreview) -> String {
    let patch = &preview.patch;
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
            changes.push(format!("{name}: {}", time(value, preview)));
        }
    }
    if let Some(case) = patch.case {
        changes.push(format!(
            "Filename case: {}, {}",
            case_mode_label(case.mode),
            case_scope_label(case.scope)
        ));
    }
    if changes.is_empty() {
        changes.push("No changes requested.".into());
    }
    if patch.recursive {
        changes.push("Includes descendants; children are processed before their parent.".into());
    }
    changes.join("\n")
}

/// The Case page's choice labels (metadata-controls.slint); keep both equal.
fn case_mode_label(mode: CaseMode) -> &'static str {
    match mode {
        CaseMode::Lower => "lowercase",
        CaseMode::Upper => "UPPERCASE",
        CaseMode::Mixed => "Mixed Case",
        CaseMode::MixedNameLowerExtension => "Mixed name.lower extension",
    }
}

fn case_scope_label(scope: CaseScope) -> &'static str {
    match scope {
        CaseScope::Whole => "Whole filename",
        CaseScope::Basename => "Base name",
        CaseScope::Extension => "Extension",
    }
}

fn preview_text(preview: &MetadataPreview) -> String {
    if preview.revision == 0 {
        "Draft not previewed. Review changes to inspect the current draft.".into()
    } else {
        requested(preview)
    }
}

fn preview_status(presenter: &Presenter) -> String {
    let state = &presenter.metadata;
    if state.busy {
        "Preparing preview… The previous file list is stale.".into()
    } else if state.preview_current() {
        state
            .preview
            .as_ref()
            .map(|preview| preview_text(preview))
            .unwrap_or_default()
    } else if state
        .preview
        .as_ref()
        .is_some_and(|preview| preview.revision > 0)
    {
        "Draft changed; the preview is stale. Review changes before Apply.".into()
    } else {
        "Draft not previewed. Review changes to inspect the current draft.".into()
    }
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
        // A name containing a line break must not forge another outcome line.
        text.push_str(&format!(
            "\n{}: {}",
            path_text(&item.current_path),
            if fields.is_empty() {
                "unchanged".into()
            } else {
                fields.join(", ")
            }
        ));
        if let Some(error) = &item.error {
            text.push_str(&format!("\n{}", crate::error_presentation::describe(error)));
        }
    }
    if result.processed > result.sample.len() as u64 {
        text.push_str("\nAdditional outcomes are included in the totals.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn empty_preview(revision: u64) -> MetadataPreview {
        MetadataPreview {
            id: OperationId(1),
            revision,
            patch: MetadataPatch::default(),
            localized_times: Default::default(),
            time_input_format: None,
            capabilities: MetadataCapabilities::default(),
            total: 1,
            conflicts: 0,
            unsupported: 0,
            hard_linked: 0,
            sample: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn case_review_uses_the_choice_labels_and_outcomes_escape_line_breaks() {
        let mut preview = empty_preview(1);
        preview.patch.case = Some(CaseChange {
            mode: CaseMode::MixedNameLowerExtension,
            scope: CaseScope::Basename,
        });
        assert_eq!(
            preview_text(&preview),
            "Filename case: Mixed name.lower extension, Base name"
        );
        let forged = std::path::PathBuf::from("x\n/home/u/.ssh/id_rsa: permissions");
        let text = result_detail(&MetadataResult {
            id: OperationId(1),
            processed: 1,
            changed: 0,
            skipped: 0,
            failed: 0,
            sample: vec![MetadataOutcome::unchanged(forged)],
        });
        assert_eq!(
            text.lines().count(),
            4,
            "one totals line and one outcome line: {text}"
        );
        assert!(
            !text.contains("\n/home/u/.ssh/id_rsa: permissions"),
            "{text}"
        );
    }

    #[test]
    fn initial_inspection_does_not_claim_the_live_draft_has_no_changes() {
        let preview = empty_preview(0);
        let text = preview_text(&preview);
        assert!(text.contains("Draft not previewed"), "{text}");
        assert!(!text.contains("No changes requested"), "{text}");
    }

    #[test]
    fn stale_or_pending_drafts_do_not_reuse_a_completed_no_change_summary() {
        let mut presenter = Presenter::new(".".into(), ".".into());
        presenter.metadata.id = Some(OperationId(1));
        presenter.metadata.revision = 1;
        presenter.metadata.preview = Some(Arc::new(empty_preview(1)));
        assert!(preview_status(&presenter).contains("No changes requested"));
        presenter.dirty_metadata("1");
        assert!(preview_status(&presenter).contains("stale"));
        assert!(!preview_status(&presenter).contains("No changes requested"));
        presenter.metadata.busy = true;
        assert!(preview_status(&presenter).contains("Preparing preview"));
    }

    #[test]
    fn metadata_review_uses_worker_localized_time_without_reinterpreting_it() {
        let mut preview = empty_preview(1);
        preview.patch.modified = TimeEdit::Set(UNIX_EPOCH);
        preview.localized_times.insert(
            UNIX_EPOCH,
            LocalizedDateTime {
                date: "1. 1. 1970".into(),
                time: "1:00:00".into(),
            },
        );
        assert_eq!(preview_text(&preview), "Modified: 1. 1. 1970 1:00:00");
        // Times without a worker-localized value use the UTC fallback.
        preview.patch.modified = TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(3600));
        assert_eq!(preview_text(&preview), "Modified: 1970-01-01 01:00:00 UTC");
    }
}
