//! Exact tree selection capture feeds the existing metadata preview and confirmation dialog.
use super::*;

pub(super) struct Pending {
    capture: tree_capture::Capture,
    case: bool,
    roots: Option<usize>,
    defer_retry: bool,
    sources: Option<Arc<dyn CapturedMetadataSources>>,
}
impl Presenter {
    pub(super) fn request_tree_metadata(
        &mut self,
        pane: PaneId,
        case: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing
            || !self.native_action_allowed(pane)
            || !self.panes[pane.index()].search_allows_activation()
        {
            return;
        }
        let Some(capture) = tree_capture::Capture::new(&self.panes[pane.index()]) else {
            return;
        };
        self.cancel_tree_rename(None);
        self.cancel_tree_delete(None);
        self.cancel_tree_metadata(None);
        self.operations.tree_metadata = Some(Pending {
            capture,
            case,
            roots: None,
            defer_retry: false,
            sources: None,
        });
        self.flush_tree_metadata(send);
    }
    pub(super) fn cancel_tree_metadata(&mut self, pane: Option<PaneId>) {
        if self
            .operations
            .tree_metadata
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.capture.view.stamp.pane == pane))
        {
            let pending = self
                .operations
                .tree_metadata
                .take()
                .expect("matched metadata capture");
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
        }
    }
    pub(super) fn flush_tree_metadata(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.operations.tree_metadata.take() else {
            return;
        };
        let p = &mut self.panes[pending.capture.view.stamp.pane.index()];
        if !p
            .tree_view()
            .is_some_and(|view| Arc::ptr_eq(view, &pending.capture.view))
        {
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
            return;
        }
        match pending.capture.status(p, self.operations.closing) {
            tree_capture::Status::Gone | tree_capture::Status::SelectionChanged => {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                return;
            }
            tree_capture::Status::Ready(request, publication) => {
                if pending.defer_retry {
                    pending.defer_retry = false;
                    self.operations.tree_metadata = Some(pending);
                    return;
                }
                let roots = publication
                    .selection
                    .len()
                    .max(usize::from(pending.capture.fallback.is_some()));
                if send(Command::Tree(Box::new(TreeCommand::Metadata(
                    TreeMetadataRequest {
                        view: pending.capture.view.clone(),
                        request,
                        selection_revision: publication.selection_revision,
                        selection: publication.selection,
                        fallback: pending.capture.fallback,
                        cancel: pending.capture.cancel.clone(),
                    },
                )))) {
                    p.tree.request = request;
                    pending.capture.request = Some(request);
                    pending.roots = Some(roots);
                }
            }
            tree_capture::Status::Waiting => {}
        }
        if let Some(sources) = &pending.sources {
            let source_directory = p.path.clone();
            if let Some(id) = self.next_operation_id(send)
                && send(Command::PrepareCapturedMetadata(PrepareCapturedMetadata {
                    id,
                    sources: sources.clone(),
                    source_directory: source_directory.clone(),
                }))
            {
                self.register_preparing_operation(id, MutationKind::EditMetadata, source_directory);
                self.retire_metadata();
                self.metadata = metadata::MetadataState {
                    id: Some(id),
                    start_case: pending.case,
                    busy: true,
                    ..Default::default()
                };
                self.retired.push(Box::new(pending));
                return;
            }
        }
        self.operations.tree_metadata = Some(pending);
    }
    pub(super) fn tree_metadata_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        retryable: bool,
        result: Result<Arc<dyn CapturedMetadataSources>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(pending) = self.operations.tree_metadata.as_mut().filter(|pending| {
            pending.capture.view.stamp == stamp
                && pending.capture.request == Some(request)
                && pending.sources.is_none()
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(sources)
                if sources.stamp() == stamp
                    && sources.request() == request
                    && pending.roots == Some(sources.root_count())
                    && (1..=MAX_METADATA_TARGETS).contains(&sources.root_count())
                    && sources.covered_roots().len() == sources.root_count() =>
            {
                pending.sources = Some(sources);
            }
            Ok(sources) => {
                self.retired.push(Box::new(sources));
                self.cancel_tree_metadata(None);
                self.desktop
                    .report("Metadata", "The captured metadata targets changed.".into());
            }
            Err(_) if retryable => {
                pending.capture.request = None;
                pending.defer_retry = true;
                // Retry on the next input pump, preserving the original selection revision.
                return;
            }
            Err(error) => {
                self.cancel_tree_metadata(None);
                self.desktop.report("Metadata", error.as_str().into());
            }
        }
        self.flush_tree_metadata(send);
    }
}
