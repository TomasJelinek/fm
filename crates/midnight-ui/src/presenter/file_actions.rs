//! Native menu and clipboard intents retain their original paths and selection snapshots.
use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MenuAnchor {
    pub(crate) owner: usize,
    pub(crate) position: ShellMenuPosition,
    pub(crate) logical: (f32, f32),
    pub(crate) modifiers: ShellMenuModifiers,
}

#[derive(Clone)]
pub(super) struct MenuContext {
    pane: PaneId,
    navigation: NavigationId,
    displayed: NavigationId,
    destination: PathBuf,
    focused: PathBuf,
    kind: EntryKind,
    anchor: MenuAnchor,
    target: ShellMenuTarget,
}

struct CapturedMenu {
    request: DesktopRequestId,
    context: MenuContext,
    files: SelectedFiles,
    paste_destination: PathBuf,
    fallback: bool,
}

#[derive(Default)]
pub(crate) struct FileActionsState {
    menu: Option<CapturedMenu>,
    clipboard_read: Option<(DesktopRequestId, PathBuf)>,
    pub(super) tree_clipboard: Option<tree_clipboard::Pending>,
}

impl Presenter {
    pub(crate) fn copy_files(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_clipboard(pane, FileClipboardIntent::Copy, send);
            return;
        }
        self.request_selection_action(pane, selection_actions::SelectionIntent::Copy, send);
    }
    pub(crate) fn cut_files(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_clipboard(pane, FileClipboardIntent::Move, send);
            return;
        }
        if !self.panes[pane.index()].native_capabilities() {
            self.desktop.report(
                "Cut unavailable",
                "Archive and virtual members cannot be cut. Use Copy to extract them.".into(),
            );
            return;
        }
        self.request_selection_action(pane, selection_actions::SelectionIntent::Cut, send);
    }

    pub(crate) fn paste_files(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if !self.native_action_allowed(pane) {
            return;
        }
        self.read_file_clipboard(self.panes[pane.index()].display_path().to_owned(), send);
    }

    fn read_file_clipboard(&mut self, destination: PathBuf, send: &dyn Fn(Command) -> bool) {
        if self.file_actions.clipboard_read.is_some() {
            return;
        }
        if !destination.is_absolute() {
            self.desktop.report(
                "Cannot paste files",
                "The destination must be absolute.".into(),
            );
            return;
        }
        let request = self.desktop.request();
        if send(Command::ReadFileClipboard {
            request,
            destination: destination.clone(),
        }) {
            self.file_actions.clipboard_read = Some((request, destination));
        } else {
            self.desktop
                .report("Clipboard queue busy", "Try pasting again.".into());
        }
    }

    fn menu_pending(&self) -> bool {
        self.file_actions.menu.is_some()
            || self.selection_actions.iter().flatten().any(|pending| {
                matches!(pending.intent, selection_actions::SelectionIntent::Menu(_))
            })
    }

    pub(crate) fn request_entry_menu(
        &mut self,
        pane: PaneId,
        row: usize,
        anchor: MenuAnchor,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.tree_action_allowed(pane) {
            return;
        }
        if !self.native_action_allowed(pane) {
            return;
        }
        if self.menu_pending() || !self.panes[pane.index()].search_allows_activation() {
            return;
        }
        let p = &self.panes[pane.index()];
        let Some(entry) = row
            .checked_sub(p.parent_offset())
            .and_then(|row| p.display_entry(row))
        else {
            return;
        };
        let selected = p.selection_visible(entry);
        let context = MenuContext {
            pane,
            navigation: NavigationId(p.generation),
            displayed: p.displayed_generation(),
            destination: p.display_path().to_owned(),
            focused: entry.path.clone(),
            kind: entry.kind,
            anchor,
            target: ShellMenuTarget::Items,
        };
        if !context.focused.is_absolute() {
            return;
        }
        self.active = pane;
        self.focus_display_row(pane, row);
        if selected {
            self.request_selection_action(
                pane,
                selection_actions::SelectionIntent::Menu(context),
                send,
            );
        } else {
            let files = SelectedFiles {
                selection: Arc::new(SelectionSnapshot::default()),
                view: self.panes[pane.index()].display_index().clone(),
                fallback: Some(context.focused.clone()),
            };
            let request = self.desktop.request();
            self.show_captured_menu(request, context, files, send);
        }
    }

    pub(crate) fn request_drive_menu(
        &mut self,
        pane: PaneId,
        token: &str,
        anchor: MenuAnchor,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.menu_pending() {
            return;
        }
        let Some((revision, id)) = token.split_once(':') else {
            return;
        };
        let (Ok(revision), Ok(id)) = (revision.parse::<u64>(), id.parse::<u64>()) else {
            return;
        };
        if revision != self.choices.revision {
            return;
        }
        let Some(choice) = self
            .choices
            .items
            .iter()
            .find(|choice| choice.id == id && choice.location.kind == LocationKind::Volume)
        else {
            return;
        };
        let root = choice.location.path.clone();
        if !root.is_absolute() {
            return;
        }
        let p = &self.panes[pane.index()];
        let context = MenuContext {
            pane,
            navigation: NavigationId(p.generation),
            displayed: p.displayed_generation(),
            destination: root.clone(),
            focused: root.clone(),
            kind: EntryKind::Directory,
            anchor,
            target: ShellMenuTarget::Drive,
        };
        let selection = Arc::new(SelectionSnapshot::default());
        let files = SelectedFiles {
            view: selection.basis().clone(),
            selection,
            fallback: Some(root),
        };
        self.active = pane;
        let request = self.desktop.request();
        self.show_captured_menu(request, context, files, send);
    }

    pub(super) fn show_captured_menu(
        &mut self,
        request: DesktopRequestId,
        context: MenuContext,
        files: SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.desktop.requests.remove(&request);
        // A single folder's native Paste means paste into that folder. A group
        // keeps the captured containing directory; destructive validation still
        // uses the originating panel directory in MenuContext.
        let paste_destination = if context.target == ShellMenuTarget::Drive
            || (files.selection.len() <= 1
                && matches!(
                    context.kind,
                    EntryKind::Directory | EntryKind::DirectoryLink
                )) {
            context.focused.clone()
        } else {
            context.destination.clone()
        };
        if send(Command::ShowShellMenu {
            request,
            files: files.clone(),
            owner: context.anchor.owner,
            position: context.anchor.position,
            modifiers: context.anchor.modifiers,
            target: context.target,
        }) {
            self.file_actions.menu = Some(CapturedMenu {
                request,
                context,
                files,
                paste_destination,
                fallback: false,
            });
        } else {
            self.retired.push(Box::new((context, files)));
            self.desktop.report(
                "Context menu queue busy",
                "Try opening the menu again.".into(),
            );
        }
    }

    pub(crate) fn file_menu_token(&self) -> String {
        self.file_actions
            .menu
            .as_ref()
            .filter(|menu| menu.fallback)
            .map_or_else(String::new, |menu| menu.request.0.to_string())
    }

    pub(crate) fn file_menu_anchor(&self) -> (f32, f32, bool) {
        self.file_actions
            .menu
            .as_ref()
            .filter(|menu| menu.fallback)
            .map_or((0.0, 0.0, false), |menu| {
                (
                    menu.context.anchor.logical.0,
                    menu.context.anchor.logical.1,
                    menu.context.target == ShellMenuTarget::Items,
                )
            })
    }

    pub(crate) fn dismiss_file_menu(&mut self, token: &str) {
        if !token.is_empty() && token == self.file_menu_token() {
            self.retired.push(Box::new(self.file_actions.menu.take()));
        }
    }

    pub(crate) fn file_menu_action(
        &mut self,
        token: &str,
        action: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        if token.is_empty() || token != self.file_menu_token() || !(0..=5).contains(&action) {
            return;
        }
        let menu = self
            .file_actions
            .menu
            .take()
            .expect("matched fallback menu");
        self.run_captured_menu_action(&menu, action, send);
        self.retired.push(Box::new(menu));
    }

    fn run_captured_menu_action(
        &mut self,
        menu: &CapturedMenu,
        action: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        match action {
            0 => match menu.context.kind {
                EntryKind::Directory => {
                    self.navigate(menu.context.pane, menu.context.focused.clone(), None, send);
                }
                EntryKind::DirectoryLink => {
                    self.open_native_directory(
                        menu.context.pane,
                        menu.context.focused.clone(),
                        send,
                    );
                }
                EntryKind::File | EntryKind::Symlink => {
                    self.open_native_entry(menu.context.pane, menu.context.focused.clone(), send);
                }
                EntryKind::Other => {}
            },
            1 | 5 | 6 => {
                let request = self.desktop.request();
                let files = menu.files.clone();
                let command = if action == 1 {
                    Command::CopyFiles { request, files }
                } else if action == 6 {
                    Command::CutFiles { request, files }
                } else {
                    Command::ShowSelectionProperties {
                        request,
                        selection: files.selection,
                        view: files.view,
                        fallback: files.fallback,
                    }
                };
                if send(command) {
                    self.desktop.requests.insert(request);
                } else {
                    self.desktop
                        .report("Desktop queue busy", "Try the action again.".into());
                }
            }
            2 => self.read_file_clipboard(menu.paste_destination.clone(), send),
            3 | 4 => {
                let p = &self.panes[menu.context.pane.index()];
                if menu.context.target != ShellMenuTarget::Items
                    || p.loading
                    || p.generation != menu.context.navigation.0
                    || p.displayed_generation() != menu.context.displayed
                    || p.display_path() != menu.context.destination
                {
                    self.desktop.report(
                        "Folder changed",
                        "Open the item's menu again before renaming or deleting it.".into(),
                    );
                    return;
                }
                self.start_captured_operation(
                    menu.context.pane,
                    if action == 3 {
                        MutationKind::Rename
                    } else {
                        MutationKind::Delete(DeleteMode::Trash)
                    },
                    &menu.files,
                    send,
                );
            }
            _ => {}
        }
    }

    pub(super) fn dispatch_file_action_event(
        &mut self,
        event: Event,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<Event> {
        match event {
            Event::ClipboardFilesRead {
                request,
                destination,
                result,
            } => {
                let matches = self
                    .file_actions
                    .clipboard_read
                    .as_ref()
                    .is_some_and(|(id, _)| *id == request);
                if !matches {
                    self.retired.push(Box::new((destination, result)));
                    return None;
                }
                let (_, captured) = self
                    .file_actions
                    .clipboard_read
                    .take()
                    .expect("matched clipboard request");
                if destination != captured {
                    self.retired.push(Box::new((destination, result)));
                    self.desktop
                        .report("Clipboard request changed", "Try pasting again.".into());
                    return None;
                }
                match result {
                    Ok(clipboard) => self.start_copy(clipboard, captured, send),
                    Err(error) => self.desktop.report(
                        "Cannot paste files",
                        crate::error_presentation::describe(&error),
                    ),
                }
                None
            }
            Event::ShellMenuFinished { request, result } => {
                if self
                    .file_actions
                    .menu
                    .as_ref()
                    .is_none_or(|menu| menu.request != request || menu.fallback)
                {
                    return None;
                }
                let mut menu = self.file_actions.menu.take().expect("matched native menu");
                match result {
                    Ok(ShellMenuOutcome::Unavailable) => {
                        menu.fallback = true;
                        self.file_actions.menu = Some(menu);
                        return None;
                    }
                    Ok(ShellMenuOutcome::Managed(verb)) => self.run_captured_menu_action(
                        &menu,
                        match verb {
                            ShellMenuVerb::Copy => 1,
                            ShellMenuVerb::Cut => 6,
                            ShellMenuVerb::Paste => 2,
                            ShellMenuVerb::Rename => 3,
                            ShellMenuVerb::Delete => 4,
                        },
                        send,
                    ),
                    Ok(ShellMenuOutcome::Dismissed | ShellMenuOutcome::Invoked) => {}
                    Err(error) => self.desktop.report(
                        "Context menu unavailable",
                        crate::error_presentation::describe(&error),
                    ),
                }
                self.retired.push(Box::new(menu));
                None
            }
            other => Some(other),
        }
    }

    pub(super) fn retire_file_actions(&mut self) {
        self.cancel_tree_clipboard(None);
        self.retired
            .push(Box::new(mem::take(&mut self.file_actions)));
    }
}
