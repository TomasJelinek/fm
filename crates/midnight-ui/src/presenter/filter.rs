//! Validate drafts on the backend before applying and saving per-panel filters.
use super::*;

enum Target {
    Flat(u64),
    Tree(TreeRootSnapshot, TreeSessionId),
}

pub(crate) struct FilterDialogState {
    pub(crate) open: bool,
    pub(crate) pane: PaneId,
    pub(crate) draft: PanelFilter,
    pub(crate) busy: bool,
    pub(crate) error: String,
    next: u64,
    pending: Option<u64>,
    ready: Option<FilterValidationRequest>,
    target: Option<Target>,
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
            target: None,
        }
    }
}

impl Presenter {
    /// Disable the applied filter without discarding its reusable expression.
    pub(crate) fn clear_panel_filter(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.filter_dialog.open && self.filter_dialog.pane == pane {
            self.cancel_panel_filter();
        }
        let p = &mut self.panes[pane.index()];
        if p.tree_view().is_some() {
            if p.tree.options.is_pending() {
                p.set_error(
                    "Tree update is busy. Wait for it to finish, then try again.",
                    &mut self.retired,
                );
                return;
            }
            if !p.search.panel_filter.enabled {
                return;
            }
            let mut filter = p.search.panel_filter.clone();
            filter.enabled = false;
            let options = TreeProjectionOptions {
                sort: p.sort,
                filter,
                search: p.tree_search_options(),
            };
            self.request_tree_options(pane, options, tree_options::Origin::FilterClear, send);
            return;
        }
        if !self.preferences.values.panel_filters[pane.index()].enabled {
            return;
        }
        let mut preferences = self.preferences.values.clone();
        preferences.panel_filters[pane.index()].enabled = false;
        self.update_preferences(preferences, send);
    }

    pub(crate) fn open_panel_filter(&mut self, pane: PaneId) {
        let p = &mut self.panes[pane.index()];
        if p.tree.options.is_pending() {
            p.set_error(
                "Tree update is busy. Wait for it to finish, then try again.",
                &mut self.retired,
            );
            return;
        }
        let target = p.tree_view().map_or(Target::Flat(p.generation), |view| {
            Target::Tree(view.root.clone(), view.stamp.session)
        });
        self.retired
            .push(Box::new(self.filter_dialog.target.replace(target)));
        self.filter_dialog.open = true;
        self.filter_dialog.pane = pane;
        self.filter_dialog.draft = p
            .tree
            .options
            .draft()
            .unwrap_or(&self.preferences.values.panel_filters[pane.index()])
            .clone();
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
            self.filter_dialog.error = "The command queue is busy. Try OK again.".into();
        }
    }

    pub(crate) fn cancel_panel_filter(&mut self) {
        self.filter_dialog.open = false;
        self.filter_dialog.busy = false;
        self.filter_dialog.pending = None;
        self.filter_dialog.ready = None;
        self.retired
            .push(Box::new(self.filter_dialog.target.take()));
    }

    pub(super) fn apply_filter_event(&mut self, event: Event) -> Option<Event> {
        let Event::PanelFilterValidated {
            request,
            result,
            diagnostic,
        } = event
        else {
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
                    self.filter_dialog.error = diagnostic.map_or_else(
                        || crate::error_presentation::describe(&error),
                        |diagnostic| describe_diagnostic(&request.filter.expression, &diagnostic),
                    )
                }
            }
        }
        None
    }

    pub fn flush_filters(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(request) = self.filter_dialog.ready.take() {
            let p = &self.panes[request.pane.index()];
            let current = match &self.filter_dialog.target {
                Some(Target::Tree(root, session)) => p.tree_view().is_some_and(|view| {
                    view.root.same_snapshot(root) && view.stamp.session == *session
                }),
                Some(Target::Flat(generation)) => {
                    p.generation == *generation && p.tree_view().is_none()
                }
                None => false,
            };
            if !current || p.loading {
                self.filter_dialog.error =
                    "The panel changed. Reopen the filter for the current folder.".into();
                return;
            }
            if p.tree_view().is_some() {
                let options = TreeProjectionOptions {
                    sort: p.sort,
                    filter: request.filter,
                    search: p.tree_search_options(),
                };
                if self.request_tree_options(
                    request.pane,
                    options,
                    tree_options::Origin::FilterApply,
                    send,
                ) {
                    self.cancel_panel_filter();
                } else {
                    self.filter_dialog.error = self.panes[request.pane.index()].error.clone();
                }
                return;
            }
            let mut preferences = self.preferences.values.clone();
            preferences.panel_filters[request.pane.index()] = request.filter;
            self.update_preferences(preferences, send);
            self.cancel_panel_filter();
        }
    }
}

fn describe_diagnostic(source: &str, diagnostic: &FilterDiagnostic) -> String {
    let start = source.floor_char_boundary(diagnostic.span.start.min(source.len()));
    let prefix = &source[..start];
    let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("Line {line}, column {column}: {}", diagnostic.message)
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
