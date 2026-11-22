//! Actions wait for the exact selection revision captured by their user gesture.
use super::*;

#[derive(Clone)]
pub(super) enum SelectionIntent {
    Properties,
    Copy,
    Cut,
    CopyDialog(copy_dialog::CopyContext),
    Metadata { pane: PaneId, case: bool },
    FolderSizes(PaneId),
    Checksums { pane: PaneId, directory: PathBuf },
    UserCommand(Box<user_commands::CommandCapture>),
    Menu(file_actions::MenuContext),
    Drag(DragId),
}

impl SelectionIntent {
    fn label(&self) -> &'static str {
        match self {
            Self::Properties => "Properties",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::CopyDialog(context) => {
                if context.intent == TransferIntent::Move {
                    "Move to folder"
                } else {
                    "Copy to folder"
                }
            }
            Self::Metadata { .. } => "editing attributes, times and case",
            Self::FolderSizes(_) => "calculating folder sizes",
            Self::Checksums { .. } => "calculating checksums",
            Self::UserCommand(_) => "running a Custom Command",
            Self::Menu(_) => "Context menu",
            Self::Drag(_) => "dragging files",
        }
    }

    /// The label as a sentence's subject.
    fn subject(&self) -> String {
        let mut label = self.label().chars();
        label
            .next()
            .map(|first| first.to_uppercase().chain(label).collect())
            .unwrap_or_default()
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
        // The selection and focus belong to the folder hidden under search
        // results fed to the panel: what the list offers acts on its marked
        // results, as its own keys do; the rest is refused.
        if self.feed(pane).is_some() {
            use super::find::feed::FeedAction;
            let action = match &intent {
                SelectionIntent::Properties => FeedAction::Properties,
                SelectionIntent::Copy => FeedAction::Clipboard { cut: false },
                SelectionIntent::Cut => FeedAction::Clipboard { cut: true },
                SelectionIntent::CopyDialog(context) => FeedAction::Transfer(context.intent),
                _ => {
                    let message = find::feed::refusal(&intent.subject());
                    self.report_selection_action_error(&intent, find::feed::REFUSED, message);
                    return;
                }
            };
            self.feed_action(pane, action, send);
            return;
        }
        if !self.tree_action_allowed(pane) || !self.native_action_allowed(pane) {
            return;
        }
        // A quick search never stops an action (`Presenter::panel_command` closes it);
        // a filtered view that is still being computed does.
        if !self.panes[pane.index()].rows_settled() {
            return;
        }
        if self.selection_actions[pane.index()].is_some() {
            self.report_selection_action_error(
                &intent,
                "Selection action waiting",
                "An action is already waiting for selection.".into(),
            );
            return;
        }
        let p = &self.panes[pane.index()];
        if p.selection_pending_bulk || p.selection_deferred.any() || p.selection_rebase_pending {
            self.report_selection_action_error(
                &intent,
                "Selection still loading",
                format!(
                    "Wait for the folder listing and selection to finish before {}.",
                    intent.label()
                ),
            );
            return;
        }
        // The focused row only while the panel shows it: a panel filter may hide it.
        let fallback = (!p.parent_cursor && p.focus_actionable())
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
            && !matches!(
                intent,
                SelectionIntent::FolderSizes(_) | SelectionIntent::UserCommand(_)
            )
        {
            self.note_filter_hides_focus(pane);
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
        // Stops the folder count a metadata summary continues with.
        let properties = matches!(intent, SelectionIntent::Properties).then(Cancellation::default);
        let command = match intent {
            SelectionIntent::Properties => Command::ShowSelectionProperties {
                request,
                selection: files.selection,
                view: files.view,
                fallback: files.fallback,
                cancel: properties.clone().unwrap_or_default(),
            },
            SelectionIntent::Copy => Command::CopyFiles { request, files },
            SelectionIntent::Cut => Command::CutFiles { request, files },
            SelectionIntent::CopyDialog(context) => {
                self.desktop.requests.remove(&request);
                let source = context.source.clone();
                self.open_captured_copy(request, context, files);
                self.inspect_copy_draft_links(Some(source), send);
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
            SelectionIntent::Checksums { pane, directory } => {
                self.desktop.requests.remove(&request);
                self.open_captured_checksums(pane, directory, files, send);
                return;
            }
            SelectionIntent::UserCommand(capture) => {
                self.desktop.requests.remove(&request);
                self.submit_user_command(*capture, Some(files), send);
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
            if let Some(cancel) = properties {
                self.properties_requested(request, cancel);
            }
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
                if deferred.any() {
                    self.desktop.requests.remove(&pending.request);
                    self.report_selection_action_error(
                        &pending.intent,
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
                let pending = self.selection_actions[pane.index()]
                    .take()
                    .expect("matched selection action");
                self.desktop.requests.remove(&pending.request);
                self.report_selection_action_error(
                    &pending.intent,
                    "Selection action cancelled",
                    "Selection could not finish. Try the action again.".into(),
                );
                self.retired.push(Box::new(pending));
            }
            _ => {}
        }
    }

    fn report_selection_action_error(
        &mut self,
        intent: &SelectionIntent,
        title: &str,
        message: String,
    ) {
        if let SelectionIntent::UserCommand(capture) = intent {
            self.report_user_command_error(capture.settings_preview.as_ref(), title, message);
        } else {
            self.desktop.report(title, message);
        }
    }

    pub(super) fn cancel_selection_actions(&mut self, pane: PaneId) {
        if let Some(pending) = self.selection_actions[pane.index()].take() {
            self.desktop.requests.remove(&pending.request);
            if let SelectionIntent::UserCommand(capture) = &pending.intent
                && capture
                    .settings_preview
                    .as_ref()
                    .is_some_and(|origin| origin.matches(&self.user_command_settings))
            {
                self.user_command_settings.clear_status();
            }
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn retire_selection_actions(&mut self) {
        self.cancel_selection_actions(PaneId::Left);
        self.cancel_selection_actions(PaneId::Right);
    }
}
