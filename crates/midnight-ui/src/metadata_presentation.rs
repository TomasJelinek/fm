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
    install_time_codec(window);
    install_now(window, state);
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
        // The values the fields started from, as set_time_defaults showed them.
        let starting = p
            .metadata
            .preview
            .as_ref()
            .map(|preview| preview.time_defaults)
            .unwrap_or_default();
        let forms = system_forms(p.metadata.preview.as_deref());
        let patch = metadata_input::patch(&draft, &starting, &p.metadata.now_values, &forms);
        let offsets = metadata_input::typed_offsets(&draft);
        p.preview_metadata(token.as_str(), patch, offsets, s);
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
    set_time_defaults(window, state.preview.as_deref());
    window.set_metadata_start_case(state.start_case);
    window.set_metadata_revision(state.revision.to_string().into());
    // Preview errors name the paths they concern.
    window.set_metadata_error(shown_text(&state.error));
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
        window.set_metadata_summary(selection_summary(preview).into());
        window.set_metadata_preview_text(preview_status(presenter).into());
        window.set_metadata_list_status(
            if preview.patch.recursive {
                "Reviewed. Subfolder contents are checked while applying."
            } else {
                "Reviewed. Apply it from the review, or change the draft and review again."
            }
            .into(),
        );
    } else {
        window.set_metadata_summary("Checking the selected items…".into());
        window.set_metadata_preview_text("".into());
        window.set_metadata_list_status("Checking the selected items…".into());
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

/// Plain header: the item's name or count, then only the issues that need attention.
fn selection_summary(preview: &MetadataPreview) -> String {
    let count = |count: u64, one: &str, many: &str| {
        if count == 1 {
            one.to_string()
        } else {
            format!("{count} {many}")
        }
    };
    let mut lines = vec![match preview.sample.as_slice() {
        [row] if preview.total == 1 => crate::isolated_name(&path_text(std::path::Path::new(
            row.path.file_name().unwrap_or(row.path.as_os_str()),
        ))),
        _ => format!("{} items selected", preview.total),
    }];
    if preview.conflicts > 0 {
        lines.push(format!(
            "{} already taken. You will be asked whether to skip {}.",
            count(preview.conflicts, "One new name is", "new names are"),
            if preview.conflicts == 1 { "it" } else { "them" }
        ));
    }
    if preview.unsupported > 0 {
        lines.push(format!(
            "{} be changed this way; see the details below.",
            count(preview.unsupported, "One item cannot", "items cannot")
        ));
    }
    if preview.hard_linked > 0 {
        lines.push("Other hard links to these files change too.".into());
    }
    lines.join("\n")
}

/// The system's date and time forms from the worker-captured edit format, if any.
fn system_forms(preview: Option<&MetadataPreview>) -> metadata_time::SystemForms {
    preview
        .and_then(|preview| preview.time_input_format.as_ref())
        .map(|format| {
            metadata_time::SystemForms::new(&format.pattern, [&format.am_pm[0], &format.am_pm[1]])
        })
        .unwrap_or_default()
}

/// Starting values for the time fields: each field's shared current value, else now.
/// Without worker-local values, now is shown in UTC with an explicit +00:00. Values
/// start as year-month-day and 24-hour times; the system's forms are accepted too.
fn set_time_defaults(window: &MainWindow, preview: Option<&MetadataPreview>) {
    let defaults = preview
        .map(|preview| preview.time_defaults)
        .unwrap_or_default();
    let now = defaults
        .now
        .map(|now| metadata_time::texts(&now))
        .unwrap_or_else(|| metadata_time::utc_texts(std::time::SystemTime::now()));
    let value = |field: Option<LocalCalendarTime>| {
        let texts = field.map_or_else(|| now.clone(), |value| metadata_time::texts(&value));
        MetadataTimeValue {
            date: texts.date.into(),
            time: texts.time.into(),
            offset: texts.offset.into(),
        }
    };
    let codec = window.global::<MetadataTimeCodec>();
    codec.set_created_default(value(defaults.created));
    codec.set_modified_default(value(defaults.modified));
    codec.set_accessed_default(value(defaults.accessed));
    codec.set_now_default(value(None));
    let format = preview.and_then(|preview| preview.time_input_format.as_ref());
    codec.set_edit_format(format.map_or_else(MetadataEditFormat::default, |format| {
        MetadataEditFormat {
            pattern: format.pattern.as_str().into(),
            am: format.am_pm[0].as_str().into(),
            pm: format.am_pm[1].as_str().into(),
        }
    }));
    let forms = system_forms(preview);
    codec.set_date_forms(metadata_time::date_forms(&forms).into());
    codec.set_time_forms(metadata_time::time_forms(&forms).into());
}

fn install_time_codec(window: &MainWindow) {
    use metadata_time::Field;
    let codec = window.global::<MetadataTimeCodec>();
    // The system forms published by `set_time_defaults` with the starting values.
    let forms = {
        let window = window.as_weak();
        move || {
            let Some(window) = window.upgrade() else {
                return metadata_time::SystemForms::default();
            };
            let format = window.global::<MetadataTimeCodec>().get_edit_format();
            metadata_time::SystemForms::new(&format.pattern, [&format.am, &format.pm])
        }
    };
    codec.on_step({
        let forms = forms.clone();
        move |text, field, cursor, delta, fallback| {
            let Some(field) = Field::from_index(field) else {
                return MetadataTimeStep::default();
            };
            let forms = forms();
            let cursor = usize::try_from(cursor).unwrap_or_default();
            let part = metadata_time::part_under(&text, field, cursor, &forms);
            step_result(metadata_time::step(
                &text, field, part, delta, &fallback, &forms,
            ))
        }
    });
    codec.on_step_part({
        let forms = forms.clone();
        move |text, field, part, delta, fallback| {
            let Some(field) = Field::from_index(field) else {
                return MetadataTimeStep::default();
            };
            let part = usize::try_from(part).unwrap_or_default();
            step_result(metadata_time::step_picker(
                &text,
                field,
                part,
                delta,
                &fallback,
                &forms(),
            ))
        }
    });
    codec.on_type_time({
        let forms = forms.clone();
        move |text, part, pending, key, fallback| {
            let part = usize::try_from(part).unwrap_or_default();
            metadata_time::type_part(&text, part, &pending, &key, &fallback, &forms())
                .map(|typed| MetadataTimeStep {
                    text: typed.text.into(),
                    start: typed.range.start as i32,
                    end: typed.range.end as i32,
                    valid: true,
                    pending: typed.pending.into(),
                    complete: typed.complete,
                })
                .unwrap_or_default()
        }
    });
    codec.on_edge_time({
        let forms = forms.clone();
        move |text, part, last, fallback| {
            let part = usize::try_from(part).unwrap_or_default();
            step_result(metadata_time::edge_part(
                &text,
                part,
                last,
                &fallback,
                &forms(),
            ))
        }
    });
    codec.on_parts({
        let forms = forms.clone();
        move |text, field, fallback| {
            let parts = Field::from_index(field)
                .map(|field| metadata_time::parts(&text, field, &fallback, &forms()))
                .unwrap_or_default();
            ModelRc::new(VecModel::from(
                parts
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            ))
        }
    });
    codec.on_month({
        let forms = forms.clone();
        move |cursor, fallback| {
            metadata_time::month_view(&cursor, &fallback, &forms())
                .map(|view| MetadataMonthView {
                    title: view.title.into(),
                    year: view.year,
                    month: view.month as i32,
                    leading: view.leading as i32,
                    days: view.days as i32,
                    day: view.day as i32,
                    valid: true,
                })
                .unwrap_or_default()
        }
    });
    codec.on_shift({
        let forms = forms.clone();
        move |cursor, fallback, days, months| {
            metadata_time::shift(&cursor, &fallback, days, months, &forms())
                .unwrap_or_else(|| cursor.to_string())
                .into()
        }
    });
    codec.on_restyle(move |date, like| {
        metadata_time::restyle(&date, &like, &forms()).map_or(date, SharedString::from)
    });
    codec.on_compose(|year, month, day| {
        metadata_time::compose(year, month, day)
            .unwrap_or_default()
            .into()
    });
}

/// Now fills a row with the current moment at the worker's latest local offset (UTC
/// when there is none) and remembers that exact instant for the row: Apply sets it,
/// sub-seconds included, unless the row's fields were edited since.
fn install_now(window: &MainWindow, state: &Rc<RefCell<Presenter>>) {
    let state = Rc::downgrade(state);
    window.global::<MetadataTimeCodec>().on_now(move |row| {
        let Some(state) = state.upgrade() else {
            return MetadataTimeValue::default();
        };
        let Ok(mut presenter) = state.try_borrow_mut() else {
            return MetadataTimeValue::default();
        };
        let offset = presenter
            .metadata
            .preview
            .as_ref()
            .and_then(|preview| preview.time_defaults.now)
            .map_or(0, |now| now.offset_seconds);
        let Some(now) = metadata_time::calendar_at(std::time::SystemTime::now(), offset) else {
            return MetadataTimeValue::default();
        };
        if let Some(slot) = usize::try_from(row)
            .ok()
            .and_then(|row| presenter.metadata.now_values.get_mut(row))
        {
            *slot = Some(now);
        }
        let texts = metadata_time::texts(&now);
        MetadataTimeValue {
            date: texts.date.into(),
            time: texts.time.into(),
            offset: texts.offset.into(),
        }
    });
}

fn step_result(step: Option<(String, std::ops::Range<usize>)>) -> MetadataTimeStep {
    step.map(|(text, range)| MetadataTimeStep {
        text: text.into(),
        start: range.start as i32,
        end: range.end as i32,
        valid: true,
        ..Default::default()
    })
    .unwrap_or_default()
}

pub(super) fn show_conflict(window: &MainWindow, conflict: &MetadataConflict, force: bool) {
    let token = format!("metadata:{}:{}", conflict.id.0, conflict.token);
    if !force && window.get_metadata_conflict_token() == token {
        return;
    }
    window.set_metadata_conflict_body(shown_text(&format!(
        "Source: {}\nRequested name: {}\n\nThe destination already exists. No existing entry will be overwritten. Skipping leaves this item's remaining changes unapplied; completed changes elsewhere are retained.",
        path_text(&conflict.source), path_text(&conflict.destination))));
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

/// A set time whose typed UTC offset is not the local one at that instant shows another
/// local hour than the one typed, typically after moving a date into or out of
/// daylight-saving time. Offsets compare in the whole minutes the fields show.
fn offset_note(
    name: &str,
    typed: Option<i32>,
    local: Option<&LocalCalendarTime>,
) -> Option<String> {
    let (typed, local) = (typed? / 60, local?);
    let local_offset = local.offset_seconds / 60;
    (typed != local_offset).then(|| {
        format!(
            "{name} uses UTC offset {}, but local time on {:04}-{:02}-{:02} is UTC{}: \
             the file will show {:02}:{:02}:{:02} local time.",
            metadata_time::offset_text(typed),
            local.year,
            local.month,
            local.day,
            metadata_time::offset_text(local_offset),
            local.hour,
            local.minute,
            local.second
        )
    })
}

fn requested(preview: &MetadataPreview, typed_offsets: &[Option<i32>; 3]) -> String {
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
    for ((name, value), typed) in [
        ("Created", patch.created),
        ("Modified", patch.modified),
        ("Accessed", patch.accessed),
    ]
    .into_iter()
    .zip(typed_offsets)
    {
        if let TimeEdit::Set(value) = value {
            changes.push(format!("{name}: {}", time(value, preview)));
            changes.extend(offset_note(
                name,
                *typed,
                preview.patch_calendars.get(&value),
            ));
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

fn preview_text(preview: &MetadataPreview, typed_offsets: &[Option<i32>; 3]) -> String {
    if preview.revision == 0 {
        "Draft not previewed. Review changes to inspect the current draft.".into()
    } else {
        requested(preview, typed_offsets)
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
            .map(|preview| preview_text(preview, &state.typed_offsets))
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
            patch_calendars: Default::default(),
            time_input_format: None,
            time_defaults: Default::default(),
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
            preview_text(&preview, &[None; 3]),
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
    fn header_names_a_single_item_and_mentions_only_issues_that_exist() {
        let mut preview = empty_preview(1);
        preview.sample.push(MetadataPreviewRow {
            path: "/home/u/Žluťoučký report.txt".into(),
            proposed_name: None,
            current: MetadataSnapshot::default(),
            proposed: MetadataSnapshot::default(),
            issue: None,
        });
        // A name on its own line keeps a left-to-right base direction.
        assert_eq!(
            selection_summary(&preview),
            "\u{2066}Žluťoučký report.txt\u{2069}"
        );
        preview.total = 3;
        assert_eq!(selection_summary(&preview), "3 items selected");
        preview.conflicts = 1;
        preview.unsupported = 2;
        preview.hard_linked = 1;
        assert_eq!(
            selection_summary(&preview),
            "3 items selected\n\
             One new name is already taken. You will be asked whether to skip it.\n\
             2 items cannot be changed this way; see the details below.\n\
             Other hard links to these files change too."
        );
        preview.conflicts = 2;
        assert!(selection_summary(&preview).contains("2 new names are already taken"));
        let forged = std::path::PathBuf::from("a\nb.txt");
        preview.total = 1;
        preview.sample[0].path = forged;
        assert_eq!(
            selection_summary(&preview).lines().count(),
            4,
            "a name cannot add lines"
        );
    }

    #[test]
    fn initial_inspection_does_not_claim_the_live_draft_has_no_changes() {
        let preview = empty_preview(0);
        let text = preview_text(&preview, &[None; 3]);
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
        assert_eq!(
            preview_text(&preview, &[None; 3]),
            "Modified: 1. 1. 1970 1:00:00"
        );
        // Times without a worker-localized value use the UTC fallback.
        preview.patch.modified = TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(3600));
        assert_eq!(
            preview_text(&preview, &[None; 3]),
            "Modified: 1970-01-01 01:00:00 UTC"
        );
    }

    #[test]
    fn review_says_when_a_kept_offset_is_not_local_time_on_the_new_date() {
        // Modified was 2026-07-15 10:00 at +02:00; only its date moved to December.
        let set = UNIX_EPOCH + Duration::from_secs(1_797_321_600);
        let mut preview = empty_preview(1);
        preview.patch.modified = TimeEdit::Set(set);
        preview.localized_times.insert(
            set,
            LocalizedDateTime {
                date: "15. 12. 2026".into(),
                time: "9:00:00".into(),
            },
        );
        let winter = LocalCalendarTime {
            year: 2026,
            month: 12,
            day: 15,
            hour: 9,
            offset_seconds: 3600,
            ..Default::default()
        };
        preview.patch_calendars.insert(set, winter);
        assert_eq!(
            preview_text(&preview, &[None, Some(7200), None]),
            "Modified: 15. 12. 2026 9:00:00\n\
             Modified uses UTC offset +02:00, but local time on 2026-12-15 is UTC+01:00: \
             the file will show 09:00:00 local time."
        );
        assert_eq!(
            preview_text(&preview, &[None, Some(3600), None]),
            "Modified: 15. 12. 2026 9:00:00",
            "a matching offset needs no note"
        );
        assert_eq!(
            preview_text(&preview, &[Some(7200), None, None]),
            "Modified: 15. 12. 2026 9:00:00",
            "each offset belongs to its own field"
        );
        // A seconds-precision local offset is compared as the fields show it.
        preview.patch_calendars.insert(
            set,
            LocalCalendarTime {
                offset_seconds: 3464,
                ..winter
            },
        );
        assert_eq!(
            preview_text(&preview, &[None, Some(57 * 60), None]),
            "Modified: 15. 12. 2026 9:00:00"
        );
    }
}
