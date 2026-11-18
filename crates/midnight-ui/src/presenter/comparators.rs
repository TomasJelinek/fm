//! Comparator drafts and accepted metadata. Compilation, discovery and I/O are backend work.
use super::comparator_draft::{Draft, RowId};
use super::*;

#[derive(Default)]
pub(crate) struct Capabilities {
    pub(crate) snapshot: Option<ComparatorCapabilities>,
    pub(crate) display: Option<Arc<CapabilityDisplay>>,
    pub(crate) pending: bool,
    pub(crate) requested: bool,
    pub(crate) error: String,
}

/// Formatted only when a new metadata record is accepted, never for draft edits.
pub(crate) struct CapabilityDisplay {
    pub(crate) coverage: slint::SharedString,
    pub(crate) status: slint::SharedString,
    pub(crate) error: slint::SharedString,
}

impl CapabilityDisplay {
    fn new(snapshot: &ComparatorCapabilities) -> Self {
        match &snapshot.images {
            Err(error) => Self {
                coverage: Default::default(),
                status: "Image support is unavailable.".into(),
                error: error.as_str().into(),
            },
            Ok(providers) if providers.is_empty() => Self {
                coverage: Default::default(),
                status: "No image decoder is enabled for this session.".into(),
                error: Default::default(),
            },
            Ok(providers) => {
                let mut extensions = providers
                    .iter()
                    .flat_map(|provider| provider.extensions.iter())
                    .map(|extension| format!("*.{}", extension.trim_start_matches('.')))
                    .collect::<Vec<_>>();
                extensions.sort_unstable();
                extensions.dedup();
                let status = if extensions.is_empty() {
                    "Enabled image decoders advertise no filename extensions."
                } else {
                    "Coverage comes from image decoders enabled for this session. Some file pairs may still be unsupported. Plugin changes take effect after restart."
                };
                Self {
                    coverage: extensions.join("; ").into(),
                    status: status.into(),
                    error: Default::default(),
                }
            }
        }
    }
}

struct Submitted {
    epoch: u64,
    revision: PreferencesRevision,
    rows: Vec<RowId>,
}

pub(crate) struct State {
    pub(crate) accepted: Option<FileComparatorSnapshot>,
    accepted_save_revision: Option<PreferencesRevision>,
    pub(crate) draft: Option<Draft>,
    pub(crate) dirty: bool,
    pub(crate) error: String,
    pub(crate) error_field: i32,
    pub(crate) focus_revision: i32,
    pub(crate) revision: u64,
    pub(crate) edit_revision: u64,
    pub(crate) epoch: u64,
    pub(crate) capabilities: Capabilities,
    submitted: Option<Submitted>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            accepted: None,
            accepted_save_revision: None,
            draft: None,
            dirty: false,
            error: String::new(),
            error_field: -1,
            focus_revision: 0,
            revision: 0,
            edit_revision: 0,
            epoch: 0,
            capabilities: Capabilities::default(),
            submitted: None,
        }
    }
}

impl Presenter {
    pub(crate) fn comparator_settings_ready(&self) -> bool {
        self.editor_preferences_ready()
            && self.comparators.accepted.is_some()
            && !self.operations.closing
    }

    fn comparator_editable(&self) -> bool {
        self.comparator_settings_ready()
            && self.preferences.options.open
            && !self.preferences.options.accepting
            && !self.preferences.options.saving
            && !self.settings_picker.busy()
    }

    pub(crate) fn begin_comparator_options(&mut self) {
        self.comparators.epoch = self.comparators.epoch.saturating_add(1);
    }

    pub(crate) fn discard_comparator_draft(&mut self) {
        self.settings_picker.invalidate();
        if let Some(draft) = self.comparators.draft.take() {
            self.retired.push(Box::new(draft));
        }
        self.comparators.submitted = None;
        self.comparators.dirty = false;
        self.comparators.error.clear();
        self.comparators.error_field = -1;
        self.comparators.edit_revision = self.comparators.edit_revision.saturating_add(1);
        self.comparators.revision = self.comparators.revision.saturating_add(1);
    }

    fn ensure_comparator_draft(&mut self) -> bool {
        if !self.comparator_editable() {
            return false;
        }
        if self.comparators.draft.is_none() {
            self.comparators.draft = Some(Draft::from_preferences(
                &self.preferences.values.file_comparators,
            ));
        }
        true
    }

    fn comparator_draft_changed(&mut self, dirty: bool) {
        self.settings_picker.invalidate();
        self.comparators.submitted = None;
        self.comparators.dirty |= dirty;
        self.comparators.error.clear();
        self.comparators.error_field = -1;
        self.comparators.edit_revision = self.comparators.edit_revision.saturating_add(1);
        self.comparators.revision = self.comparators.revision.saturating_add(1);
    }

    pub(crate) fn select_comparator_rule(&mut self, row: i32) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        if self.ensure_comparator_draft()
            && self
                .comparators
                .draft
                .as_mut()
                .is_some_and(|draft| draft.select(row))
        {
            self.comparator_draft_changed(false);
        }
    }

    pub(crate) fn comparator_rule_action(&mut self, action: i32) {
        if self.ensure_comparator_draft()
            && self
                .comparators
                .draft
                .as_mut()
                .is_some_and(|draft| draft.action(action))
        {
            self.comparator_draft_changed(true);
        }
    }

    pub(crate) fn edit_comparator_form(&mut self, form: crate::ComparatorOptionsDraft) {
        if self.ensure_comparator_draft()
            && self
                .comparators
                .draft
                .as_mut()
                .is_some_and(|draft| draft.edit_selected(form))
        {
            self.comparator_draft_changed(true);
        }
    }

    pub(crate) fn edit_comparator_argument(&mut self, operation: i32, row: i32, value: &str) {
        if self.ensure_comparator_draft()
            && self
                .comparators
                .draft
                .as_mut()
                .is_some_and(|draft| draft.change_arguments(operation, row, value))
        {
            self.comparator_draft_changed(true);
        }
    }

    /// Stage only raw values into the existing Options transaction. Its accepted
    /// Arc/key remains unchanged until a successful backend acknowledgement.
    pub(crate) fn stage_comparator_draft(&mut self, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(draft) = self.comparators.draft.as_ref() else {
            return true;
        };
        if !self.comparators.dirty {
            return true;
        }
        let candidate = match draft.candidate() {
            Ok(candidate) => candidate,
            Err(diagnostic) => {
                self.comparator_diagnostic(None, &diagnostic);
                return false;
            }
        };
        self.preferences.edited.file_comparators = true;
        let mut values = self.preferences.values.clone();
        values.file_comparators = candidate;
        self.update_preferences(values, send);
        self.comparators.error.clear();
        self.comparators.error_field = -1;
        self.comparators.revision = self.comparators.revision.saturating_add(1);
        true
    }

    pub(crate) fn comparator_submission(&mut self, revision: PreferencesRevision) {
        self.comparators.submitted = self
            .comparators
            .draft
            .as_ref()
            .filter(|_| self.comparators.dirty)
            .map(|draft| Submitted {
                epoch: self.comparators.epoch,
                revision,
                rows: draft.row_order(),
            });
    }

    /// A diagnostic index refers to the submitted order, never a newer draft.
    pub(crate) fn comparator_diagnostic(
        &mut self,
        revision: Option<PreferencesRevision>,
        diagnostic: &FileComparatorDiagnostic,
    ) -> bool {
        let Some(draft) = self.comparators.draft.as_ref() else {
            return false;
        };
        let order = if let Some(revision) = revision {
            let Some(submitted) = self.comparators.submitted.as_ref().filter(|submitted| {
                submitted.epoch == self.comparators.epoch && revision >= submitted.revision
            }) else {
                return false;
            };
            &submitted.rows
        } else {
            // Local Core diagnostics are produced synchronously by this draft.
            let row = diagnostic
                .rule
                .and_then(|index| draft.rules.get(index))
                .map(|row| row.id);
            if diagnostic.rule.is_some() && row.is_none() {
                return false;
            }
            return self.show_comparator_diagnostic(row, diagnostic);
        };
        let row = diagnostic.rule.and_then(|index| order.get(index)).copied();
        if diagnostic.rule.is_some() && row.is_none() {
            return false;
        }
        self.show_comparator_diagnostic(row, diagnostic)
    }

    fn show_comparator_diagnostic(
        &mut self,
        row: Option<RowId>,
        diagnostic: &FileComparatorDiagnostic,
    ) -> bool {
        let Some(draft) = self.comparators.draft.as_mut() else {
            return false;
        };
        if let Some(id) = row {
            if !draft.rules.iter().any(|rule| rule.id == id) {
                return false;
            }
            draft.selected = Some(id);
        } else {
            draft.selected = None;
        }
        self.settings_picker.invalidate();
        self.comparators.error = diagnostic.message.clone();
        self.comparators.error_field = field_index(&diagnostic.field);
        self.comparators.focus_revision = self.comparators.focus_revision.saturating_add(1);
        self.comparators.revision = self.comparators.revision.saturating_add(1);
        true
    }

    /// Every successful committed acknowledgement is authoritative, including
    /// one older than the latest local edit. Rejected drafts never arrive here.
    pub(crate) fn comparator_preferences_accepted(
        &mut self,
        snapshot: FileComparatorSnapshot,
        revision: Option<PreferencesRevision>,
    ) {
        if let Some(revision) = revision {
            if self
                .comparators
                .accepted_save_revision
                .is_some_and(|seen| revision < seen)
            {
                self.retired.push(Box::new(snapshot));
                return;
            }
            self.comparators.accepted_save_revision = Some(revision);
        }
        let same = self.comparators.accepted.as_ref().is_some_and(|accepted| {
            accepted.id == snapshot.id && Arc::ptr_eq(&accepted.preferences, &snapshot.preferences)
        });
        if same {
            self.retired.push(Box::new(snapshot));
            return;
        }
        if let Some(old) = self.comparators.accepted.replace(snapshot) {
            self.retired.push(Box::new(old));
        }
        self.comparators.revision = self.comparators.revision.saturating_add(1);
    }

    pub(crate) fn comparator_picker_context(&self) -> Option<(u64, u64, RowId)> {
        let draft = self.comparators.draft.as_ref()?;
        let rule = draft.selected_rule()?;
        (self.comparator_editable() && rule.target == super::comparator_draft::Target::External)
            .then_some((
                self.comparators.epoch,
                self.comparators.edit_revision,
                rule.id,
            ))
    }

    pub(crate) fn comparator_path_picked(&mut self, path: PathBuf, directory: bool) {
        if self
            .comparators
            .draft
            .as_mut()
            .is_some_and(|draft| draft.picked(path, directory))
        {
            self.comparator_draft_changed(true);
        }
    }

    pub(crate) fn request_comparator_capabilities(
        &mut self,
        explicit: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.comparators.capabilities.pending
            || (!explicit && self.comparators.capabilities.requested)
            || self.operations.closing
            || self.comparators.accepted.is_none()
        {
            return;
        }
        let Some(disabled_plugins) = self.viewer_plugins.startup_disabled.clone() else {
            return;
        };
        self.comparators.capabilities.requested = true;
        if send(Command::ListComparatorCapabilities { disabled_plugins }) {
            self.comparators.capabilities.pending = true;
            self.comparators.capabilities.error.clear();
        } else {
            self.comparators.capabilities.error =
                "Image support metadata is busy. Try Refresh again.".into();
        }
        self.comparators.revision = self.comparators.revision.saturating_add(1);
    }

    pub(crate) fn comparator_capabilities_listed(&mut self, snapshot: ComparatorCapabilities) {
        let current = self
            .viewer_plugins
            .startup_disabled
            .as_ref()
            .is_some_and(|disabled| disabled.as_ref() == snapshot.disabled_plugins.as_ref());
        let newer = self
            .comparators
            .capabilities
            .snapshot
            .as_ref()
            .is_none_or(|old| snapshot.revision > old.revision);
        if !current || !newer {
            self.retired.push(Box::new(snapshot));
            return;
        }
        let display = Arc::new(CapabilityDisplay::new(&snapshot));
        self.comparators.capabilities.pending = false;
        self.comparators.capabilities.requested = true;
        self.comparators.capabilities.error.clear();
        let old = (
            self.comparators.capabilities.snapshot.replace(snapshot),
            self.comparators.capabilities.display.replace(display),
        );
        if old.0.is_some() || old.1.is_some() {
            self.retired.push(Box::new(old));
        }
        self.comparators.revision = self.comparators.revision.saturating_add(1);
    }
}

fn field_index(field: &FileComparatorField) -> i32 {
    match field {
        FileComparatorField::Rules => 0,
        FileComparatorField::PatternMode => 1,
        FileComparatorField::Expression => 2,
        FileComparatorField::Executable => 3,
        FileComparatorField::Arguments => 4,
        FileComparatorField::WorkingDirectory => 5,
    }
}

#[cfg(test)]
#[path = "comparators_tests.rs"]
mod tests;
