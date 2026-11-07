//! Bounded metadata forms and conflict presentation; native work is sent to the engine.
use super::*;
use std::cell::Cell;

pub(super) fn bind(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) {
    window.set_metadata_rows(ModelRc::new(MetadataRows {
        presenter: state.clone(),
        revision: Cell::new(None),
        notify: ModelNotify::default(),
    }));
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
    window.set_metadata_preview_current(state.preview_current());
    if let Some(rows) = window
        .get_metadata_rows()
        .as_any()
        .downcast_ref::<MetadataRows>()
    {
        let revision = state
            .preview
            .as_ref()
            .map(|preview| (preview.id, preview.revision));
        if rows.revision.replace(revision) != revision {
            rows.notify.reset();
        }
    }
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
    if preview.revision == 0 {
        "Draft not previewed. Refresh or Preview changes to inspect the current draft.".into()
    } else {
        requested(&preview.patch)
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
        "Draft changed; the preview is stale. Refresh or Preview changes before Apply.".into()
    } else {
        "Draft not previewed. Refresh or Preview changes to inspect the current draft.".into()
    }
}

struct MetadataRows {
    presenter: Rc<RefCell<Presenter>>,
    revision: Cell<Option<(OperationId, u64)>>,
    notify: ModelNotify,
}

impl Model for MetadataRows {
    type Data = MetadataFileRow;
    fn row_count(&self) -> usize {
        self.presenter
            .borrow()
            .metadata
            .preview
            .as_ref()
            .map_or(0, |preview| preview.sample.len().min(MAX_METADATA_TARGETS))
    }
    fn row_data(&self, index: usize) -> Option<Self::Data> {
        if index >= self.row_count() {
            return None;
        }
        let presenter = self.presenter.borrow();
        let preview = presenter.metadata.preview.as_ref()?;
        preview
            .sample
            .get(index)
            .map(|row| file_row(row, &preview.patch))
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn attributes(snapshot: &MetadataSnapshot) -> String {
    let mut text = Vec::new();
    if let Some(value) = snapshot.windows_attributes {
        let flags = [
            (EDIT_READ_ONLY, "R"),
            (EDIT_HIDDEN, "H"),
            (EDIT_SYSTEM, "S"),
            (EDIT_ARCHIVE, "A"),
        ]
        .into_iter()
        .filter_map(|(flag, label)| (value & flag != 0).then_some(label))
        .collect::<Vec<_>>();
        text.push(if flags.is_empty() {
            "None".into()
        } else {
            flags.join(" ")
        });
    }
    if let Some(mode) = snapshot.unix_mode {
        text.push(format!("{:04o}", mode & 0o7777));
    }
    for (label, value) in [
        ("Compressed", snapshot.compressed),
        ("Encrypted", snapshot.encrypted),
    ] {
        if let Some(value) = value {
            text.push(format!("{label}: {}", if value { "yes" } else { "no" }));
        }
    }
    if text.is_empty() {
        "Unavailable".into()
    } else {
        text.join(" · ")
    }
}

fn file_row(row: &MetadataPreviewRow, patch: &MetadataPatch) -> MetadataFileRow {
    let name = row.path.file_name().unwrap_or_default().to_string_lossy();
    let proposed = row
        .proposed_name
        .as_deref()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| name.clone());
    let attributes = format!(
        "Current: {}\nProposed: {}",
        attributes(&row.current),
        attributes(&row.proposed)
    );
    let times = [
        ("Modified", row.current.modified, patch.modified),
        ("Created", row.current.created, patch.created),
        ("Accessed", row.current.accessed, patch.accessed),
    ]
    .into_iter()
    .map(|(label, before, edit)| {
        let before = before.map(time).unwrap_or_else(|| "Unavailable".into());
        match edit {
            TimeEdit::Set(value) => format!("{label}: {before} → {}", time(value)),
            TimeEdit::Keep => format!("{label}: {before} (unchanged)"),
            TimeEdit::Now => format!("{label}: {before} → Now (awaiting preview)"),
        }
    })
    .collect::<Vec<_>>()
    .join("\n");
    let issue = row
        .issue
        .as_ref()
        .map(|issue| format!("\n{}", crate::error_presentation::describe(issue)))
        .unwrap_or_default();
    MetadataFileRow {
        current_name: name.as_ref().into(), proposed_name: proposed.as_ref().into(),
        detail: format!("{}\nProposed name: {proposed}\n{attributes}\n{times}\nR = Read-only · H = Hidden · S = System · A = Archive{issue}", row.path.display()).into(),
        attributes: attributes.into(), times: times.into(), issue: row.issue.is_some(),
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
    fn virtual_rows_preserve_unicode_and_show_requested_metadata_at_the_last_root() {
        let row = MetadataPreviewRow {
            path: "ŽLUŤOUČKÝ.TXT".into(),
            proposed_name: Some("žluťoučký.txt".into()),
            current: MetadataSnapshot {
                windows_attributes: Some(EDIT_HIDDEN),
                modified: Some(UNIX_EPOCH),
                ..Default::default()
            },
            proposed: MetadataSnapshot {
                windows_attributes: Some(EDIT_READ_ONLY),
                modified: Some(UNIX_EPOCH + Duration::from_secs(3600)),
                ..Default::default()
            },
            issue: None,
        };
        let mut preview = empty_preview(1);
        preview.patch = MetadataPatch {
            windows_mask: EDIT_READ_ONLY | EDIT_HIDDEN,
            windows_value: EDIT_READ_ONLY,
            modified: TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(3600)),
            ..Default::default()
        };
        preview.sample = vec![row; MAX_METADATA_TARGETS];
        let mut presenter = Presenter::new(".".into(), ".".into());
        presenter.metadata.preview = Some(Arc::new(preview));
        let rows = MetadataRows {
            presenter: Rc::new(RefCell::new(presenter)),
            revision: Cell::new(None),
            notify: ModelNotify::default(),
        };
        assert_eq!(rows.row_count(), 256);
        let last = rows.row_data(255).unwrap();
        assert_eq!(last.current_name.as_str(), "ŽLUŤOUČKÝ.TXT");
        assert_eq!(last.proposed_name.as_str(), "žluťoučký.txt");
        assert_eq!(last.attributes.as_str(), "Current: H\nProposed: R");
        assert!(
            last.times
                .contains("1970-01-01 00:00:00 UTC → 1970-01-01 01:00:00 UTC")
        );
        assert!(rows.row_data(256).is_none());
    }
}
