//! Native menus retain the clicked occurrence and selection until a verb is chosen.
use super::tree_clipboard::{Pending, Purpose};
use super::*;

pub(super) struct Menu {
    pub row: TreeRowId,
    pub kind: EntryKind,
    pub anchor: file_actions::MenuAnchor,
    pub single: bool,
    fallback: bool,
}

impl Presenter {
    pub(super) fn request_tree_properties(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        self.request_tree_action(
            pane,
            FileClipboardIntent::Copy,
            TreeClipboardScope::SelectionOrFallback,
            Purpose::Properties,
            send,
        );
    }

    pub(super) fn request_tree_entry_menu(
        &mut self,
        pane: PaneId,
        position: usize,
        anchor: file_actions::MenuAnchor,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing || self.menu_pending() || !self.native_action_allowed(pane) {
            return;
        }
        if self.file_actions.tree_clipboard.is_some() {
            self.desktop.report(
                "File action pending",
                "Wait for the current file action to finish.".into(),
            );
            return;
        }
        let p = &self.panes[pane.index()];
        if !p.tree_input_ready() || !p.search_allows_activation() {
            return;
        }
        let Some((row, kind)) = position
            .checked_sub(p.parent_offset())
            .and_then(|position| p.tree_row(position))
            .filter(|row| row.selectable && !row.context_only)
            .map(|row| (row.id, row.entry.kind))
        else {
            return;
        };
        self.active = pane;
        self.focus_display_row(pane, position);
        self.request_tree_action(
            pane,
            FileClipboardIntent::Copy,
            TreeClipboardScope::Dragged(row),
            Purpose::Menu(Menu {
                row,
                kind,
                anchor,
                single: false,
                fallback: false,
            }),
            send,
        );
    }

    fn tree_menu(&self) -> Option<(&Pending, &Menu)> {
        self.file_actions
            .tree_clipboard
            .as_ref()
            .and_then(|pending| match &pending.purpose {
                Purpose::Menu(menu) => Some((pending, menu)),
                _ => None,
            })
    }

    pub(super) fn tree_menu_pending(&self) -> bool {
        self.tree_menu().is_some()
    }

    pub(super) fn tree_menu_token(&self) -> Option<String> {
        self.tree_menu()
            .filter(|(_, menu)| menu.fallback)
            .and_then(|(pending, _)| pending.desktop_request.map(|request| request.0.to_string()))
    }

    pub(super) fn tree_menu_anchor(&self) -> Option<(f32, f32, bool)> {
        self.tree_menu()
            .filter(|(_, menu)| menu.fallback)
            .map(|(_, menu)| (menu.anchor.logical.0, menu.anchor.logical.1, true))
    }

    pub(super) fn dismiss_tree_menu(&mut self, token: &str) -> bool {
        if token.is_empty() || self.tree_menu_token().as_deref() != Some(token) {
            return false;
        }
        self.cancel_tree_clipboard(None);
        true
    }

    pub(super) fn try_tree_menu_action(
        &mut self,
        token: &str,
        action: i32,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if token.is_empty() || self.tree_menu_token().as_deref() != Some(token) {
            return false;
        }
        if (0..=5).contains(&action) {
            let pending = self
                .file_actions
                .tree_clipboard
                .take()
                .expect("matched fallback tree menu");
            self.run_tree_menu_action(pending, action, send);
        }
        true
    }

    pub(super) fn is_tree_menu_request(&self, request: DesktopRequestId) -> bool {
        self.tree_menu().is_some_and(|(pending, menu)| {
            pending.desktop_request == Some(request) && !menu.fallback
        })
    }

    pub(super) fn tree_menu_finished(
        &mut self,
        request: DesktopRequestId,
        result: Result<ShellMenuOutcome, FsError>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.is_tree_menu_request(request) {
            return;
        }
        let mut pending = self
            .file_actions
            .tree_clipboard
            .take()
            .expect("matched native tree menu");
        match result {
            Ok(ShellMenuOutcome::Unavailable) => {
                if let Purpose::Menu(menu) = &mut pending.purpose {
                    menu.fallback = true;
                }
                self.file_actions.tree_clipboard = Some(pending);
                return;
            }
            Ok(ShellMenuOutcome::Managed(verb)) => {
                let action = match verb {
                    ShellMenuVerb::Copy => 1,
                    ShellMenuVerb::Cut => 6,
                    ShellMenuVerb::Paste => 2,
                    ShellMenuVerb::Rename => 3,
                    ShellMenuVerb::Delete => 4,
                };
                self.run_tree_menu_action(pending, action, send);
                return;
            }
            Err(error) => self.desktop.report(
                "Context menu unavailable",
                crate::error_presentation::describe(&error),
            ),
            Ok(ShellMenuOutcome::Dismissed | ShellMenuOutcome::Invoked) => {}
        }
        pending.capture.cancel.cancel();
        self.retired.push(Box::new(pending));
    }

    fn run_tree_menu_action(
        &mut self,
        mut pending: Pending,
        action: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Purpose::Menu(menu) = &pending.purpose else {
            self.retired.push(Box::new(pending));
            return;
        };
        let (row, kind, single) = (menu.row, menu.kind, menu.single);
        let pane = pending.capture.view.stamp.pane;
        let p = &self.panes[pane.index()];
        if self.operations.closing
            || !p.native_capabilities()
            || !p.search_allows_activation()
            || !pending.capture.matches_current(p)
        {
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
            self.desktop.report(
                "Folder or selection changed",
                "Open the item's menu again before choosing an action.".into(),
            );
            return;
        }
        match action {
            0 | 5 => {
                pending.purpose = if action == 0 {
                    Purpose::Open(row)
                } else {
                    Purpose::Properties
                };
                pending.desktop_request = None;
                self.file_actions.tree_clipboard = Some(pending);
                self.flush_tree_clipboard(send);
            }
            1 | 6 | 4 => {
                let Some(capture) = pending.capture.fork_if_current(p) else {
                    pending.capture.cancel.cancel();
                    self.retired.push(Box::new(pending));
                    return;
                };
                let scope = pending.scope;
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                if action == 4 {
                    self.start_tree_delete_capture(DeleteMode::Trash, capture, scope, send);
                } else {
                    self.start_tree_action(
                        capture,
                        if action == 6 {
                            FileClipboardIntent::Move
                        } else {
                            FileClipboardIntent::Copy
                        },
                        scope,
                        Purpose::Clipboard,
                        send,
                    );
                }
            }
            2 if single && matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink) => {
                pending.purpose = Purpose::Paste;
                pending.desktop_request = None;
                self.file_actions.tree_clipboard = Some(pending);
                self.flush_tree_clipboard(send);
            }
            2 => {
                let destination = p.display_path().to_owned();
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                self.read_file_clipboard(destination, send);
            }
            3 => {
                let view = pending.capture.view.clone();
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                self.request_tree_rename_target(pane, view, row, send);
            }
            _ => {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
            }
        }
    }

    pub(super) fn tree_open_request(
        &self,
        request: DesktopRequestId,
    ) -> Option<(PaneId, NavigationId)> {
        self.file_actions
            .tree_clipboard
            .as_ref()
            .filter(|pending| {
                pending.desktop_request == Some(request)
                    && matches!(pending.purpose, Purpose::Open(_))
                    && !pending.capture.cancel.is_cancelled()
            })
            .map(|pending| {
                (
                    pending.capture.view.stamp.pane,
                    pending.capture.view.stamp.navigation,
                )
            })
    }

    pub(super) fn is_tree_paste_request(&self, request: DesktopRequestId) -> bool {
        self.file_actions
            .tree_clipboard
            .as_ref()
            .is_some_and(|pending| {
                pending.desktop_request == Some(request)
                    && matches!(pending.purpose, Purpose::Paste)
            })
    }

    pub(super) fn tree_paste_finished(
        &mut self,
        request: DesktopRequestId,
        destination: PathBuf,
        result: Result<FileClipboard, FsError>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.is_tree_paste_request(request) {
            self.retired.push(Box::new((destination, result)));
            return;
        }
        let pending = self
            .file_actions
            .tree_clipboard
            .take()
            .expect("matched captured Paste");
        self.desktop.requests.remove(&request);
        let pane = pending.capture.view.stamp.pane;
        if matches!(
            pending
                .capture
                .status(&self.panes[pane.index()], self.operations.closing),
            tree_capture::Status::Gone
        ) {
            self.retired.push(Box::new((pending, destination, result)));
            return;
        }
        match result {
            Ok(clipboard)
                if destination.is_absolute()
                    && !destination.as_os_str().as_encoded_bytes().contains(&0) =>
            {
                self.start_copy(clipboard, destination, send);
            }
            Ok(clipboard) => {
                self.retired.push(Box::new(clipboard));
                self.desktop.report(
                    "Cannot paste files",
                    "The captured destination is invalid.".into(),
                );
            }
            Err(error) => self.desktop.report(
                "Cannot paste files",
                crate::error_presentation::describe(&error),
            ),
        }
        self.retired.push(Box::new(pending));
    }
}
