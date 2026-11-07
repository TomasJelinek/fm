//! Validate drafts on the backend before applying and saving per-panel filters.
use super::*;

pub(crate) struct FilterDialogState {
    pub(crate) open: bool,
    pub(crate) pane: PaneId,
    pub(crate) draft: PanelFilter,
    pub(crate) busy: bool,
    pub(crate) error: String,
    next: u64,
    pending: Option<u64>,
    ready: Option<FilterValidationRequest>,
}

impl Default for FilterDialogState {
    fn default() -> Self {
        Self {
            open: false,
            pane: PaneId::Left,
            draft: PanelFilter::default(),
            busy: false,
            error: String::new(),
            next: 0,
            pending: None,
            ready: None,
        }
    }
}

impl Presenter {
    pub(crate) fn open_panel_filter(&mut self, pane: PaneId) {
        self.filter_dialog.open = true;
        self.filter_dialog.pane = pane;
        self.filter_dialog.draft = self.preferences.values.panel_filters[pane.index()].clone();
        self.filter_dialog.error.clear();
        self.filter_dialog.busy = false;
        self.filter_dialog.pending = None;
        self.filter_dialog.ready = None;
    }

    pub(crate) fn submit_panel_filter(
        &mut self,
        pane: PaneId,
        filter: PanelFilter,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.filter_dialog.open || self.filter_dialog.pane != pane || self.filter_dialog.busy {
            return;
        }
        self.filter_dialog.draft = filter.clone();
        self.filter_dialog.error.clear();
        if let Err(error) = filter.validate_bounds() {
            self.filter_dialog.error = error.into();
            return;
        }
        let Some(id) = self.filter_dialog.next.checked_add(1) else {
            self.filter_dialog.error = "Filter identities are exhausted.".into();
            return;
        };
        if send(Command::ValidatePanelFilter(FilterValidationRequest {
            id,
            pane,
            filter,
        })) {
            self.filter_dialog.next = id;
            self.filter_dialog.pending = Some(id);
            self.filter_dialog.busy = true;
        } else {
            self.filter_dialog.error = "The command queue is busy. Try Apply again.".into();
        }
    }

    pub(crate) fn cancel_panel_filter(&mut self) {
        self.filter_dialog.open = false;
        self.filter_dialog.busy = false;
        self.filter_dialog.pending = None;
        self.filter_dialog.ready = None;
    }

    pub(super) fn apply_filter_event(&mut self, event: Event) -> Option<Event> {
        let Event::PanelFilterValidated { request, result } = event else {
            return Some(event);
        };
        if self.filter_dialog.open
            && self.filter_dialog.pending == Some(request.id)
            && self.filter_dialog.pane == request.pane
        {
            self.filter_dialog.pending = None;
            self.filter_dialog.busy = false;
            match result {
                Ok(()) => self.filter_dialog.ready = Some(request),
                Err(error) => {
                    self.filter_dialog.error = crate::error_presentation::describe(&error)
                }
            }
        }
        None
    }

    pub fn flush_filters(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(request) = self.filter_dialog.ready.take() {
            let mut preferences = self.preferences.values.clone();
            preferences.panel_filters[request.pane.index()] = request.filter;
            self.update_preferences(preferences, send);
            self.filter_dialog.open = false;
        }
    }
}
