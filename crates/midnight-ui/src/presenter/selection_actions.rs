//! Actions wait for the exact selection revision captured by their user gesture.
use super::*;

#[derive(Clone)]
pub(super) enum SelectionIntent {
    Properties,
    Copy,
    CopyDialog(copy_dialog::CopyContext),
    Metadata { pane: PaneId, case: bool },
    FolderSizes(PaneId),
    Menu(file_actions::MenuContext),
    Drag(DragId),
}

impl SelectionIntent {
    fn label(&self) -> &'static str {
        match self {
            Self::Properties => "Properties",
            Self::Copy => "Copy",
            Self::CopyDialog(_) => "Copy to folder",
            Self::Metadata { .. } => "editing attributes, times and case",
            Self::FolderSizes(_) => "calculating folder sizes",
            Self::Menu(_) => "Context menu",
            Self::Drag(_) => "dragging files",
        }
    }
}

pub(super) struct PendingSelectionAction {
    request: DesktopRequestId,
    navigation: NavigationId,
    revision: SelectionRevision,
    view: Option<Arc<DirectoryIndex>>,
    fallback: Option<PathBuf>,
    pub(super) intent: SelectionIntent,
}

impl Presenter {
    pub(super) fn request_selection_action(
        &mut self,
        pane: PaneId,
        intent: SelectionIntent,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.native_action_allowed(pane) {
            return;
        }
        if !self.panes[pane.index()].search_allows_activation() {
            return;
        }
        if self.selection_actions[pane.index()].is_some() {
            self.desktop.report(
                "Selection action waiting",
                "An action is already waiting for selection.".into(),
            );
            return;
        }
        let p = &self.panes[pane.index()];
        if p.selection_pending_bulk || p.selection_deferred || p.selection_rebase_pending {
            self.desktop.report(
                "Selection still loading",
                format!(
                    "Wait for the folder listing and selection to finish before {}.",
                    intent.label()
                ),
            );
            return;
        }
        let fallback = (!p.parent_cursor)
            .then(|| {
                p.cursor
                    .and_then(|id| p.entry_by_id(id))
                    .map(|entry| entry.path.clone())
            })
            .flatten();
        let view = (!p.showing_stream()).then(|| p.display_index().clone());
        let pending = p.selection_revision > p.selection_resolved;
        let selection = p.selected.clone();
        let navigation = NavigationId(p.generation);
        let revision = p.selection_revision;
        if !pending
            && selection.is_empty()
            && fallback.is_none()
            && !matches!(intent, SelectionIntent::FolderSizes(_))
        {
            return;
        }
        let request = self.desktop.request();
        if pending {
            self.desktop.requests.insert(request);
            self.selection_actions[pane.index()] = Some(PendingSelectionAction {
                request,
                navigation,
                revision,
                view,
                fallback,
                intent,
            });
        } else {
            self.dispatch_selected_action(
                request,
                intent,
                SelectedFiles {
                    view: view.unwrap_or_else(|| selection.basis().clone()),
                    selection,
                    fallback,
                },
                send,
            );
        }
    }

    fn dispatch_selected_action(
        &mut self,
        request: DesktopRequestId,
        intent: SelectionIntent,
        files: SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) {
        let label = intent.label();
        let command = match intent {
            SelectionIntent::Properties => Command::ShowSelectionProperties {
                request,
                selection: files.selection,
                view: files.view,
                fallback: files.fallback,
            },
            SelectionIntent::Copy => Command::CopyFiles { request, files },
            SelectionIntent::CopyDialog(context) => {
                self.desktop.requests.remove(&request);
                self.open_captured_copy(request, context, files);
                return;
            }
            SelectionIntent::Metadata { pane, case } => {
                self.desktop.requests.remove(&request);
                self.start_captured_metadata(pane, case, &files, send);
                return;
            }
            SelectionIntent::FolderSizes(pane) => {
                self.desktop.requests.remove(&request);
                self.start_captured_folder_sizes(pane, files.selection, send);
                return;
            }
            SelectionIntent::Menu(context) => {
                self.show_captured_menu(request, context, files, send);
                return;
            }
            SelectionIntent::Drag(id) => {
                self.prepare_selected_drag(request, id, files, send);
                return;
            }
        };
        if send(command) {
            self.desktop.requests.insert(request);
        } else {
            self.desktop.requests.remove(&request);
            self.desktop.report(
                &format!("{label} queue busy"),
                "Try the action again.".into(),
            );
        }
    }

    pub(super) fn dispatch_selection_actions(
        &mut self,
        event: &Event,
        send: &dyn Fn(Command) -> bool,
    ) {
        match event {
            Event::SelectionChanged {
                pane,
                navigation,
                revision,
                selection,
                deferred,
            } => {
                let slot = pane.index();
                let matches = self.selection_actions[slot]
                    .as_ref()
                    .is_some_and(|pending| {
                        pending.navigation == *navigation
                            && pending.revision == *revision
                            && self.panes[slot].generation == navigation.0
                    });
                if !matches {
                    return;
                }
                let pending = self.selection_actions[slot]
                    .take()
                    .expect("matched selection action");
                if *deferred {
                    self.desktop.requests.remove(&pending.request);
                    self.desktop.report(
                        "Selection still loading",
                        "Try the action after selection finishes.".into(),
                    );
                } else {
                    self.dispatch_selected_action(
                        pending.request,
                        pending.intent.clone(),
                        SelectedFiles {
                            selection: selection.clone(),
                            view: pending
                                .view
                                .clone()
                                .unwrap_or_else(|| selection.basis().clone()),
                            fallback: pending.fallback.clone(),
                        },
                        send,
                    );
                }
                self.retired.push(Box::new(pending));
            }
            Event::SelectionFailed {
                pane,
                navigation,
                revision,
                ..
            } if self.selection_actions[pane.index()]
                .as_ref()
                .is_some_and(|pending| {
                    pending.navigation == *navigation && pending.revision <= *revision
                }) =>
            {
                self.cancel_selection_actions(*pane);
                self.desktop.report(
                    "Selection action cancelled",
                    "Selection could not finish. Try the action again.".into(),
                );
            }
            _ => {}
        }
    }

    pub(super) fn cancel_selection_actions(&mut self, pane: PaneId) {
        if let Some(pending) = self.selection_actions[pane.index()].take() {
            self.desktop.requests.remove(&pending.request);
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn retire_selection_actions(&mut self) {
        self.cancel_selection_actions(PaneId::Left);
        self.cancel_selection_actions(PaneId::Right);
    }
}
