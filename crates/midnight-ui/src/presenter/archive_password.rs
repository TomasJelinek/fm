//! The password of a mounted archive, asked once before an explicit action needs it:
//! F3 on an encrypted member, Enter on an encrypted archive inside the archive, F5 of
//! a selection that holds encrypted members, a listing whose names are encrypted, and
//! Enter archive password…. The typed text goes straight into the `ArchivePassword`
//! of `Command::UnlockArchive`; the Presenter keeps none of it.
//!
//! The action is bound to what the dialog was opened for: its pane, the archive session
//! that the pane showed, and the member, selection or folder that the action named. A
//! tool window can move the pane while the dialog is open (Find's Go to file): the action
//! goes on only while the pane still shows that session, and always on what it captured,
//! never on the row under the cursor by then.
use super::copy_dialog::CopyDraft;
use super::*;

/// The message under the field after a refused password.
pub(crate) const WRONG_PASSWORD: &str = "Wrong password — try again.";

/// What goes on once the dialog closes, with what it was opened for.
pub(crate) enum PasswordAction {
    /// F3 or Alt+F3 on a member: its viewer, captured when F3 asked.
    View {
        pane: PaneId,
        launch: Box<ViewerLaunch>,
    },
    /// Enter on an encrypted archive inside the archive, held by the folder `location`.
    OpenMember {
        pane: PaneId,
        location: Box<ArchiveLocation>,
        node: Box<ArchiveNode>,
        format: ArchiveFormat,
    },
    /// F5 in the archive panel: the Copy dialog of the selection that F5 named.
    Copy { pane: PaneId, draft: Box<CopyDraft> },
    /// List this folder again: its names are encrypted.
    List {
        pane: PaneId,
        location: Box<ArchiveLocation>,
    },
    /// Enter archive password…: unlocking is all.
    Unlock { pane: PaneId },
}

impl PasswordAction {
    fn pane(&self) -> PaneId {
        match self {
            Self::View { pane, .. }
            | Self::OpenMember { pane, .. }
            | Self::Copy { pane, .. }
            | Self::List { pane, .. }
            | Self::Unlock { pane } => *pane,
        }
    }

    /// What the status line says when the pane left the archive before this could go on.
    fn left(&self) -> &'static str {
        match self {
            Self::View { .. } | Self::OpenMember { .. } => {
                "The archive was left; open the member again."
            }
            Self::Copy { .. } => "The archive was left; copy the members again.",
            Self::List { .. } => "The archive was left; open the folder again.",
            Self::Unlock { .. } => "The archive was left; open it again.",
        }
    }
}

#[derive(Default)]
pub(crate) struct ArchivePasswordState {
    pub(crate) open: bool,
    /// Each opening has a new identity; stale dialog callbacks are ignored.
    pub(crate) generation: u64,
    /// The archive session that the dialog's pane showed when it opened.
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
    /// That viewer's launch as F3 captured it, for the UI's view request (`view_command`).
    unlocked_view: Option<(PaneId, Box<ViewerLaunch>)>,
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
            self.retired.push(Box::new((mount, action)));
            return;
        };
        let archive = self.panes[pane.index()]
            .archive
            .as_ref()
            .and_then(|archive| archive.location.trail.first())
            .map_or_else(|| "This archive".to_owned(), |(_, name)| name.clone());
        // A viewer that an earlier password let open still waits for the UI.
        let state = &mut self.archive_password;
        let view = state.view.take();
        let unlocked_view = state.unlocked_view.take();
        let previous = mem::replace(
            state,
            ArchivePasswordState {
                open: true,
                generation,
                mount: Some(mount),
                archive: midnight_core::visible_name(&archive).into_owned(),
                purpose,
                action: Some(action),
                view: std::cell::Cell::new(view),
                unlocked_view,
                ..ArchivePasswordState::default()
            },
        );
        self.retired.push(Box::new(previous));
    }

    /// OK: send the password to the archive's worker. The dialog stays open until its
    /// answer (`apply_archive_unlocked`); an empty field says what is missing. A pane that
    /// has left the archive meanwhile unlocks nothing (`archive_password_left`).
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
        if !self.archive_password_target_shown() {
            self.archive_password_left();
            return;
        }
        let Some(mount) = self.archive_password.mount.clone() else {
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
    /// and the dialog closes without a message. Only a copy goes on, skipping the protected
    /// members, and only in the archive that its pane still shows.
    pub(crate) fn cancel_archive_password(&mut self) {
        if !self.archive_password.open {
            return;
        }
        let shown = self.archive_password_target_shown();
        match self.close_archive_password() {
            Some(PasswordAction::Copy { draft, .. }) if shown => {
                self.open_archive_copy_draft(*draft);
            }
            Some(action @ PasswordAction::Copy { .. }) => {
                self.desktop.status = action.left().into();
                self.retired.push(Box::new(action));
            }
            action => self.retired.push(Box::new(action)),
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

    /// Whether the open dialog's pane still shows the archive session that it was opened
    /// for, with no navigation away from it in flight.
    fn archive_password_target_shown(&self) -> bool {
        let state = &self.archive_password;
        let (Some(mount), Some(action)) = (&state.mount, &state.action) else {
            return false;
        };
        let panel = &self.panes[action.pane().index()];
        panel.pending.is_none()
            && panel
                .archive
                .as_ref()
                .is_some_and(|archive| archive.location.mount_id() == Some(mount.id()))
    }

    /// The pane left the archive before the action could go on: the dialog closes, the
    /// status line says so once, and nothing is unlocked, sent or opened.
    fn archive_password_left(&mut self) {
        if let Some(action) = self.close_archive_password() {
            self.desktop.status = action.left().into();
            self.retired.push(Box::new(action));
        }
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
            Ok(ArchiveUnlock::NotEncrypted)
                if matches!(
                    self.archive_password.action,
                    None | Some(PasswordAction::Unlock { .. })
                ) =>
            {
                let action = self.close_archive_password();
                self.retired.push(Box::new(action));
                self.desktop.report(
                    "Archive password",
                    format!("{archive} has no password-protected members."),
                );
            }
            // The pane left the archive while its worker checked the password.
            Ok(ArchiveUnlock::Unlocked | ArchiveUnlock::NotEncrypted)
                if !self.archive_password_target_shown() =>
            {
                self.archive_password_left();
            }
            Ok(ArchiveUnlock::Unlocked | ArchiveUnlock::NotEncrypted) => {
                if let Some(action) = self.close_archive_password() {
                    self.after_archive_unlocked(action, send);
                }
            }
            Ok(ArchiveUnlock::Unsupported) => {
                let action = self.close_archive_password();
                self.retired.push(Box::new(action));
                self.desktop.report(
                    "Archive password",
                    format!(
                        "{archive}'s members are encrypted with a method that the archive \
                         plugin cannot decrypt."
                    ),
                );
            }
            Err(error) => {
                let action = self.close_archive_password();
                self.retired.push(Box::new(action));
                self.desktop.report(
                    "Archive password",
                    crate::error_presentation::describe(&error),
                );
            }
        }
    }

    /// Carry out `action` with the password, on what the dialog was opened for, which its
    /// pane still shows (`archive_password_target_shown`).
    fn after_archive_unlocked(&mut self, action: PasswordAction, send: &dyn Fn(Command) -> bool) {
        match action {
            PasswordAction::View { pane, launch } => {
                self.archive_password.view.set(Some((pane, launch.basic)));
                let replaced = self.archive_password.unlocked_view.replace((pane, launch));
                self.retired.push(Box::new(replaced));
            }
            PasswordAction::OpenMember {
                pane,
                location,
                node,
                format,
            } => {
                self.open_member_archive_in(pane, *location, *node, format, send);
            }
            PasswordAction::Copy { draft, .. } => self.open_archive_copy_draft(*draft),
            PasswordAction::List { pane, location } => {
                self.archive_navigate(pane, ArchiveAction::List(*location), send);
            }
            PasswordAction::Unlock { .. } => {
                let archive = self.archive_password.archive.clone();
                self.desktop.status =
                    format!("{archive}: its password-protected members can be read now.");
            }
        }
    }

    /// F3 or Alt+F3 on an encrypted member of an archive without its password yet:
    /// ask for it first. True when the dialog opened; the UI then opens the viewer
    /// that F3 named once it is unlocked (`take_archive_view`). A member that cannot be
    /// viewed now is not asked for: `view_command` says why.
    pub fn view_needs_archive_password(&mut self, pane: PaneId, basic: bool) -> bool {
        let panel = &self.panes[pane.index()];
        if panel.parent_cursor
            || panel.loading
            || self.archive_password.open
            || self.unlocked_view_waits(pane, basic)
        {
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
        let Some(launch) = self.viewer_launch(pane, basic) else {
            return false;
        };
        let name = midnight_core::visible_name(&node.display_name).into_owned();
        self.ask_archive_password(
            pane,
            mount,
            format!("Enter its password to view {name}."),
            PasswordAction::View {
                pane,
                launch: Box::new(launch),
            },
        );
        true
    }

    /// The viewer that an unlocked archive lets the UI open now, once.
    pub fn take_archive_view(&self) -> Option<(PaneId, bool)> {
        self.archive_password.view.take()
    }

    /// Whether the viewer that `take_archive_view` handed to the UI waits for its view
    /// request from `pane` (`basic` for Alt+F3).
    fn unlocked_view_waits(&self, pane: PaneId, basic: bool) -> bool {
        self.archive_password
            .unlocked_view
            .as_ref()
            .is_some_and(|(shown, launch)| *shown == pane && launch.basic == basic)
    }

    /// The launch of that viewer, as F3 captured it, for the UI's view request
    /// (`view_command`): never the row under the cursor by then.
    pub(super) fn take_unlocked_view(&mut self, pane: PaneId, basic: bool) -> Option<ViewerLaunch> {
        if !self.unlocked_view_waits(pane, basic) {
            return None;
        }
        let (_, launch) = self.archive_password.unlocked_view.take()?;
        Some(*launch)
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
            PasswordAction::Unlock { pane },
        );
    }

    /// F5 in an archive panel whose selection holds encrypted members of a locked
    /// archive: ask for the password once before `draft`, the Copy dialog of that
    /// selection, opens. Returns `draft` when no password is needed.
    pub(super) fn copy_needs_archive_password(
        &mut self,
        pane: PaneId,
        draft: CopyDraft,
    ) -> Option<CopyDraft> {
        let panel = &self.panes[pane.index()];
        let Some(archive) = &panel.archive else {
            return Some(draft);
        };
        let Some(mount) = archive
            .location
            .mount
            .clone()
            .filter(|mount| !mount.unlocked())
        else {
            return Some(draft);
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
            return Some(draft);
        }
        self.ask_archive_password(
            pane,
            mount,
            "Enter its password to copy its protected members; without it they are skipped.".into(),
            PasswordAction::Copy {
                pane,
                draft: Box::new(draft),
            },
        );
        None
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
