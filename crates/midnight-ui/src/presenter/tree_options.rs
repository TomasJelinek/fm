//! One committed options intent per pane, applied only by its correlated publication.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Origin {
    FilterApply,
    FilterClear,
    Sort,
    Preferences,
    Search,
}

struct Pending {
    request: TreeReprojectRequest,
    origin: Origin,
}

#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    retry: Option<PanelFilter>,
    preference: Option<PanelFilter>,
    pub(super) applied_search: TreeSearchOptions,
    defer_retry: bool,
}

impl State {
    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn draft(&self) -> Option<&PanelFilter> {
        self.retry.as_ref()
    }
}

impl Presenter {
    pub(super) fn request_tree_options(
        &mut self,
        pane: PaneId,
        options: TreeProjectionOptions,
        origin: Origin,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        if p.loading
            || !p.tree_projection_ready()
            || p.tree.revision != p.tree.resolved
            || !p.tree.preview.is_empty()
        {
            p.set_error(
                "Tree update is busy. Wait for it to finish, then try again.",
                &mut self.retired,
            );
            return false;
        }
        if let Err(error) = options.filter.validate_bounds() {
            p.set_error(error, &mut self.retired);
            return false;
        }
        if let Err(error) = options.search.validate_bounds() {
            p.set_error(error, &mut self.retired);
            return false;
        }
        let Some(publication) = p.tree.publication.as_ref() else {
            return false;
        };
        let Some(request) = p.tree.request.checked_next() else {
            p.set_error("Tree request identities are exhausted.", &mut self.retired);
            return false;
        };
        let request = TreeReprojectRequest {
            view: publication.view.clone(),
            request,
            selection_revision: publication.selection_revision,
            selection: publication.selection.clone(),
            options,
        };
        if !send(Command::Tree(Box::new(TreeCommand::Reproject(
            request.clone(),
        )))) {
            if !matches!(origin, Origin::Search) {
                p.set_error("The command queue is busy. Try again.", &mut self.retired);
            }
            self.retired.push(Box::new(request));
            return false;
        }
        p.tree.request = request.request;
        // A successfully queued explicit choice supersedes an older preference
        // that was still waiting for a settled view/selection.
        if !matches!(origin, Origin::Search) {
            self.retired
                .push(Box::new(p.tree.options.preference.take()));
        }
        p.tree.options.pending = Some(Pending { request, origin });
        p.set_error("", &mut self.retired);
        self.desktop.status = "Applying tree options…".into();
        true
    }

    pub(super) fn tree_options_applied(
        &mut self,
        previous: TreeViewStamp,
        request: TreeRequestId,
        options: TreeProjectionOptions,
        publication: TreePublication,
        send: &dyn Fn(Command) -> bool,
    ) {
        let pane = previous.pane;
        let p = &self.panes[pane.index()];
        let matches = p.tree.options.pending.as_ref().is_some_and(|pending| {
            pending.request.request == request
                && pending.request.view.stamp == previous
                && pending.request.options == options
                && pending
                    .request
                    .view
                    .root
                    .same_snapshot(&publication.view.root)
                && publication.view.stamp.pane == pane
                && publication.view.stamp.navigation == previous.navigation
                && publication.view.stamp.session == previous.session
                && p.tree_view().is_some_and(|view| {
                    view.stamp.session == previous.session
                        && view.root.same_snapshot(&pending.request.view.root)
                })
        });
        if !matches {
            self.retired.push(Box::new(publication));
            return;
        }
        if !self.publish_tree(publication) {
            self.tree_options_failed(
                previous,
                request,
                TreeDiagnostic::new("The tree changed before options could be applied. Try again."),
                true,
            );
            return;
        }
        let p = &mut self.panes[pane.index()];
        if let Some(pending) = p.tree.options.pending.take() {
            if !matches!(pending.origin, Origin::Sort | Origin::Search) {
                self.retired.push(Box::new(p.tree.options.retry.take()));
            }
            self.retired.push(Box::new(pending));
        }
        p.sort = options.sort;
        p.committed_sort = options.sort;
        p.search.panel_filter = options.filter.clone();
        p.set_error("", &mut self.retired);
        self.desktop.status = "Tree options applied".into();
        self.tree_search_applied(pane, options.search);
        if self.preferences.values.panel_filters[pane.index()] != options.filter {
            let mut preferences = self.preferences.values.clone();
            preferences.panel_filters[pane.index()] = options.filter;
            self.update_preferences(preferences, send);
        }
    }

    pub(super) fn tree_options_failed(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        error: TreeDiagnostic,
        retryable: bool,
    ) {
        let p = &mut self.panes[stamp.pane.index()];
        // Branch completion may have advanced the current revision. The retained
        // request stamp, not that newer revision, owns this terminal receipt.
        let matches = p.tree.options.pending.as_ref().is_some_and(|pending| {
            pending.request.request == request && pending.request.view.stamp == stamp
        });
        if !matches {
            return;
        }
        let mut retry_search = false;
        if let Some(pending) = p.tree.options.pending.take() {
            if matches!(pending.origin, Origin::FilterApply) {
                self.retired.push(Box::new(
                    p.tree
                        .options
                        .retry
                        .replace(pending.request.options.filter.clone()),
                ));
            }
            let advanced = p
                .tree_view()
                .is_some_and(|view| !Arc::ptr_eq(view, &pending.request.view));
            let changed_search = pending.request.options.search.query != p.search.query
                || (!p.search.query.is_empty()
                    && pending.request.options.search.filter_matches != p.search.filter);
            if matches!(pending.origin, Origin::Search) || p.search.pending {
                p.search.pending = retryable || advanced || changed_search;
                p.search.needs_request = p.search.pending;
                p.search.failed = !p.search.pending;
                if p.search.failed {
                    p.tree.search_insert = None;
                }
                // A full internal mailbox can reject a request after outer
                // admission. Retry on a later pump, never in a receipt loop.
                p.tree.options.defer_retry = retryable && !advanced && !changed_search;
                retry_search = matches!(pending.origin, Origin::Search) && p.search.pending;
            }
            self.retired.push(Box::new(pending));
        }
        if !retry_search {
            p.set_tree_error(&error, "", &mut self.retired);
            self.desktop.status = "Tree options could not be applied".into();
        }
    }

    /// Loading/preferences may propose a filter, but saved/applied state remains
    /// the last acknowledged value until this proposal receives a publication.
    pub(super) fn stage_tree_preference(&mut self, pane: PaneId, wanted: PanelFilter) {
        let p = &mut self.panes[pane.index()];
        self.preferences.values.panel_filters[pane.index()] = p.search.panel_filter.clone();
        self.retired
            .push(Box::new(p.tree.options.preference.replace(wanted)));
    }

    pub(super) fn flush_tree_preferences(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            let p = &mut self.panes[pane.index()];
            if p.tree_view().is_none() {
                continue;
            }
            if p.tree.options.defer_retry {
                p.tree.options.defer_retry = false;
                continue;
            }
            if p.loading
                || !p.tree_projection_ready()
                || p.tree.revision != p.tree.resolved
                || !p.tree.preview.is_empty()
            {
                continue;
            }
            if p.tree.options.preference.is_none() && !p.search.needs_request {
                continue;
            }
            let origin = if p.tree.options.preference.is_some() {
                Origin::Preferences
            } else {
                Origin::Search
            };
            let search = p.tree_search_options();
            // Escape can supersede a query before any request was admitted.
            // No backend transaction is needed when the accepted view still
            // has this empty query and there is no persistent-filter proposal.
            if p.tree.options.preference.is_none()
                && search.query.is_empty()
                && p.tree.options.applied_search.query.is_empty()
            {
                p.search.pending = false;
                p.search.needs_request = false;
                p.search.choose_first = false;
                p.search.failed = false;
                continue;
            }
            let options = TreeProjectionOptions {
                sort: p.sort,
                filter: p
                    .tree
                    .options
                    .preference
                    .as_ref()
                    .unwrap_or(&p.search.panel_filter)
                    .clone(),
                search,
            };
            if self.request_tree_options(pane, options, origin, send) {
                self.panes[pane.index()].search.needs_request = false;
            }
        }
    }
}
