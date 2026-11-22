//! The password of a mounted archive, asked once before an explicit action needs it:
//! F3 on an encrypted member, Enter on an encrypted archive inside the archive, F5 of
//! a selection that holds encrypted members, a listing whose names are encrypted, and
//! Enter archive password…. The typed text goes straight into the `ArchivePassword`
//! of `Command::UnlockArchive`; the Presenter keeps none of it.
use super::*;

/// The message under the field after a refused password.
pub(crate) const WRONG_PASSWORD: &str = "Wrong password — try again.";

/// What goes on once the dialog closes.
#[derive(Clone)]
pub(crate) enum PasswordAction {
    /// F3 or Alt+F3 (`basic`) on the focused member: the UI opens its viewer.
    View { pane: PaneId, basic: bool },
    /// Enter on an encrypted archive inside the archive.
    OpenMember {
        pane: PaneId,
        node: Box<ArchiveNode>,
        format: ArchiveFormat,
    },
    /// F5 in the archive panel: the Copy dialog.
    Copy { pane: PaneId },
    /// List this folder again: its names are encrypted.
    List {
        pane: PaneId,
        location: Box<ArchiveLocation>,
    },
    /// Enter archive password…: unlocking is all.
    Unlock,
}

#[derive(Default)]
pub(crate) struct ArchivePasswordState {
    pub(crate) open: bool,
    /// Each opening has a new identity; stale dialog callbacks are ignored.
    pub(crate) generation: u64,
    mount: Option<Arc<dyn ArchiveMount>>,
    /// The archive's shown name and what the password lets go on.
    pub(crate) archive: String,
    pub(crate) purpose: String,
    pub(crate) error: String,
    /// Refused passwords so far; the dialog selects the text after each one.
    pub(crate) attempt: i32,
    /// The unlock in flight; OK waits for its answer.
    pub(crate) pending: Option<DesktopRequestId>,
    action: Option<PasswordAction>,
    /// A viewer for the UI to open once the dialog has closed (`take_archive_view`).
    view: std::cell::Cell<Option<(PaneId, bool)>>,
}

impl Presenter {
    /// The locked mount whose password opening `node` of `pane` needs: an encrypted
    /// readable member, or a folder holding one, of a mount without a password yet.
    pub(crate) fn archive_password_needed(
        &self,
        pane: PaneId,
        node: &ArchiveNode,
    ) -> Option<Arc<dyn ArchiveMount>> {
        let mount = self.panes[pane.index()]
            .archive
            .as_ref()?
            .location
            .mount
            .as_ref()?;
        (node.needs_password() && node.id.mount == mount.id() && !mount.unlocked())
            .then(|| mount.clone())
    }

    /// Open the password dialog for `mount`, shown in `pane`, before `action`.
    pub(crate) fn ask_archive_password(
        &mut self,
        pane: PaneId,
        mount: Arc<dyn ArchiveMount>,
        purpose: String,
        action: PasswordAction,
    ) {
        let Some(generation) = self.archive_password.generation.checked_add(1) else {
            return;
        };
        let archive = self.panes[pane.index()]
            .archive
            .as_ref()
            .and_then(|archive| archive.location.trail.first())
            .map_or_else(|| "This archive".to_owned(), |(_, name)| name.clone());
        let previous = mem::replace(
            &mut self.archive_password,
            ArchivePasswordState {
                open: true,
                generation,
                mount: Some(mount),
                archive: midnight_core::visible_name(&archive).into_owned(),
                purpose,
                action: Some(action),
                ..ArchivePasswordState::default()
            },
        );
        self.retired.push(Box::new(previous.mount));
    }

    /// OK: send the password to the archive's worker. The dialog stays open until its
    /// answer (`apply_archive_unlocked`); an empty field says what is missing.
    pub(crate) fn submit_archive_password(
        &mut self,
        password: ArchivePassword,
        send: &dyn Fn(Command) -> bool,
    ) {
        let state = &mut self.archive_password;
        if !state.open || state.pending.is_some() {
            return;
        }
        if password.is_empty() {
            state.error = "Type the password first.".into();
            return;
        }
        let Some(mount) = state.mount.clone() else {
            return;
        };
        let request = self.desktop.request();
        let state = &mut self.archive_password;
        if send(Command::UnlockArchive {
            request,
            mount,
            password,
        }) {
            state.pending = Some(request);
            state.error.clear();
        } else {
            state.error = "The command queue is busy. Try OK again.".into();
        }
    }

    /// Escape, Cancel or the header's × (the main window's close request sends Escape to an
    /// open dialog): the user chose to go on without the password. The members stay locked
    /// and the dialog closes without a message (`after_archive_password`).
    pub(crate) fn cancel_archive_password(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.archive_password.open {
            return;
        }
        let action = self.close_archive_password();
        if let Some(action) = action {
            self.after_archive_password(action, false, send);
        }
    }

    fn close_archive_password(&mut self) -> Option<PasswordAction> {
        let state = &mut self.archive_password;
        state.open = false;
        state.pending = None;
        state.error.clear();
        let action = state.action.take();
        let mount = state.mount.take();
        self.retired.push(Box::new(mount));
        action
    }

    /// The worker's answer to the dialog's password.
    pub(super) fn apply_archive_unlocked(
        &mut self,
        request: DesktopRequestId,
        result: Result<ArchiveUnlock, FsError>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.archive_password.open || self.archive_password.pending != Some(request) {
            return;
        }
        self.archive_password.pending = None;
        let archive = self.archive_password.archive.clone();
        match result {
            Ok(ArchiveUnlock::WrongPassword) => {
                let state = &mut self.archive_password;
                state.error = WRONG_PASSWORD.into();
                state.attempt = state.attempt.saturating_add(1);
            }
            Ok(ArchiveUnlock::Unlocked) => {
                if let Some(action) = self.close_archive_password() {
                    self.after_archive_password(action, true, send);
                }
            }
            Ok(ArchiveUnlock::NotEncrypted) => {
                let action = self.close_archive_password();
                match action {
                    Some(PasswordAction::Unlock) | None => self.desktop.report(
                        "Archive password",
                        format!("{archive} has no password-protected members."),
                    ),
                    Some(action) => self.after_archive_password(action, true, send),
                }
            }
            Ok(ArchiveUnlock::Unsupported) => {
                self.close_archive_password();
                self.desktop.report(
                    "Archive password",
                    format!(
                        "{archive}'s members are encrypted with a method that the archive \
                         plugin cannot decrypt."
                    ),
                );
            }
            Err(error) => {
                self.close_archive_password();
                self.desktop.report(
                    "Archive password",
                    crate::error_presentation::describe(&error),
                );
            }
        }
    }

    /// Carry out `action` once the dialog has closed: `unlocked`, or dismissed by the user
    /// (`cancel_archive_password`), the only closure without the password that reaches
    /// this. A dismissal says nothing: F3, Enter, a locked listing and Enter archive
    /// password… stop, and the file list gets the keyboard back on the same row; only a
    /// copy goes on. A closure the user did not choose says why where it happens
    /// (`apply_archive_unlocked`).
    fn after_archive_password(
        &mut self,
        action: PasswordAction,
        unlocked: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        match action {
            PasswordAction::View { pane, basic } if unlocked => {
                self.archive_password.view.set(Some((pane, basic)));
            }
            PasswordAction::OpenMember { pane, node, format } if unlocked => {
                self.open_member_archive(pane, *node, format, send);
            }
            // Dismissed: the user knows that the member stays closed.
            PasswordAction::View { .. } | PasswordAction::OpenMember { .. } => {}
            // Without the password the copy goes on and skips the protected members.
            PasswordAction::Copy { pane } => self.request_archive_copy_dialog_unchecked(pane),
            PasswordAction::List { pane, location } => {
                if unlocked {
                    self.archive_navigate(pane, ArchiveAction::List(*location), send);
                }
            }
            PasswordAction::Unlock => {
                if unlocked {
                    let archive = self.archive_password.archive.clone();
                    self.desktop.status =
                        format!("{archive}: its password-protected members can be read now.");
                }
            }
        }
    }

    /// F3 or Alt+F3 on an encrypted member of an archive without its password yet:
    /// ask for it first. True when the dialog opened; the UI then opens the viewer
    /// once it is unlocked (`take_archive_view`).
    pub fn view_needs_archive_password(&mut self, pane: PaneId, basic: bool) -> bool {
        let panel = &self.panes[pane.index()];
        if panel.parent_cursor || panel.loading || self.archive_password.open {
            return false;
        }
        let Some(node) = panel
            .cursor
            .and_then(|id| panel.entry_by_id(id))
            .and_then(|entry| entry.virtual_node.as_deref())
            .filter(|node| node.kind == EntryKind::File)
            .cloned()
        else {
            return false;
        };
        let Some(mount) = self.archive_password_needed(pane, &node) else {
            return false;
        };
        let name = midnight_core::visible_name(&node.display_name).into_owned();
        self.ask_archive_password(
            pane,
            mount,
            format!("Enter its password to view {name}."),
            PasswordAction::View { pane, basic },
        );
        true
    }

    /// The viewer that an unlocked archive lets the UI open now, once.
    pub fn take_archive_view(&self) -> Option<(PaneId, bool)> {
        self.archive_password.view.take()
    }

    /// Enter archive password…: ask for the password of `pane`'s archive. It has no
    /// menu entry or shortcut yet: a new shortcut-catalog action needs the catalog's
    /// acceptance coverage first.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn request_archive_password(&mut self, pane: PaneId) {
        let mount = self.panes[pane.index()]
            .archive
            .as_ref()
            .and_then(|archive| archive.location.mount.clone());
        let Some(mount) = mount else {
            self.desktop.report(
                "Archive password",
                "Open an archive in this panel first; its password is asked for there.".into(),
            );
            return;
        };
        if mount.unlocked() {
            self.desktop.status = "This archive's password was already entered.".into();
            return;
        }
        self.ask_archive_password(
            pane,
            mount,
            "Enter its password to read its protected members.".into(),
            PasswordAction::Unlock,
        );
    }

    /// F5 in an archive panel whose selection holds encrypted members of a locked
    /// archive: ask for the password once before the Copy dialog. True when it asked.
    pub(super) fn copy_needs_archive_password(&mut self, pane: PaneId) -> bool {
        let panel = &self.panes[pane.index()];
        let Some(archive) = &panel.archive else {
            return false;
        };
        let Some(mount) = archive
            .location
            .mount
            .clone()
            .filter(|mount| !mount.unlocked())
        else {
            return false;
        };
        let needs = |entry: &Entry| {
            entry
                .virtual_node
                .as_deref()
                .is_some_and(ArchiveNode::needs_password)
        };
        let needed = if archive.selected.is_empty() {
            (!panel.parent_cursor)
                .then_some(panel.cursor)
                .flatten()
                .and_then(|id| panel.entry_by_id(id))
                .is_some_and(needs)
        } else {
            archive
                .selected
                .iter()
                .filter_map(|id| panel.entry_by_id(*id))
                .any(needs)
        };
        if !needed {
            return false;
        }
        self.ask_archive_password(
            pane,
            mount,
            "Enter its password to copy its protected members; without it they are skipped.".into(),
            PasswordAction::Copy { pane },
        );
        true
    }

    /// A listing of `location` came back empty because its names are encrypted: ask
    /// for the password, then list it again.
    pub(super) fn listing_needs_archive_password(
        &mut self,
        pane: PaneId,
        location: &ArchiveLocation,
    ) {
        let Some(mount) = location.mount.clone() else {
            return;
        };
        if self.archive_password.open || mount.unlocked() || !mount.listing_locked() {
            return;
        }
        self.ask_archive_password(
            pane,
            mount,
            "Its file names are encrypted: enter its password to list them.".into(),
            PasswordAction::List {
                pane,
                location: Box::new(location.clone()),
            },
        );
    }
}
