//! The once-per-installation notice that other accounts can change the installation folder,
//! with its buttons: secure the folder, copy Midnight Explorer into the profile, dismiss.
//! The backend does the work on workers (`Command::Installation`); this only presents it.
use super::Presenter;
use midnight_core::{Command, InstallationEvent, InstallationRequest, InstallationStatus};
use std::{cell::Cell, path::PathBuf};

const SECURE: &str = "Secure this folder";
const COPY: &str = "Move to my profile…";
const REPLACE: &str = "Replace the copy";
const KEEP: &str = "Keep the existing copy";
const RESTART: &str = "Restart from the copy";
const OPEN_ORIGINAL: &str = "Open the original folder";
const MACHINE: &str = if cfg!(windows) { "PC" } else { "computer" };

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Offer,
    /// Backend work is running; the text says which.
    Busy,
    /// The profile already holds a copy at this folder.
    AskReplace,
    /// A copy was made; restart is offered only when it is private.
    Copied { folder: PathBuf, private: bool },
    /// The folder is secured; nothing is left to offer.
    Secured,
    /// Securing left something that other accounts can change; copying remains.
    PartlySecured,
}

/// What the notice shows: its text and the buttons before Dismiss.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InstallationNoticeView {
    pub(crate) body: String,
    pub(crate) detail: String,
    pub(crate) busy: bool,
    pub(crate) actions: Vec<&'static str>,
}

#[derive(Default)]
pub(crate) struct InstallationNotice {
    status: Option<InstallationStatus>,
    phase: Phase,
    detail: String,
    /// The phase to return to when starting the copy fails.
    restart_target: Option<Phase>,
    /// The copy has started: the window closes on the next refresh.
    pub(crate) close_requested: Cell<bool>,
}

fn shown(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text
    }
}

fn body(status: &InstallationStatus) -> String {
    let what = match (&status.shared_ancestor, status.shared_count) {
        (Some(ancestor), 0) => format!(
            "Other accounts on this {MACHINE} can rename or replace {}, the folder that \
             contains Midnight Explorer.",
            shown(ancestor)
        ),
        _ => format!(
            "Other accounts on this {MACHINE} can change the files in {}.",
            shown(&status.folder)
        ),
    };
    format!(
        "{what} On a shared {MACHINE}, secure the folder or copy Midnight Explorer into your \
         profile."
    )
}

/// What other accounts can still change after securing, in a sentence or two.
fn remaining(status: &InstallationStatus, failures: &[String]) -> String {
    let mut parts = Vec::new();
    if let Some(ancestor) = &status.shared_ancestor {
        parts.push(format!(
            "Other accounts can still rename or replace {}, which contains this folder; \
             Midnight Explorer never changes folders outside its own.",
            shown(ancestor)
        ));
    }
    if status.shared_count > 0 {
        let first = status
            .shared
            .first()
            .map(|path| format!(", for example {}", shown(path)))
            .unwrap_or_default();
        parts.push(format!(
            "Other accounts can still change {} item(s) in it{first}.",
            status.shared_count
        ));
    }
    if let Some(refused) = status.refused.first() {
        parts.push(format!("Not changed: {refused}."));
    }
    if let Some(failure) = failures.first() {
        parts.push(format!("Could not change: {failure}."));
    }
    parts.push("Copy Midnight Explorer into your profile to keep it private.".into());
    parts.join(" ")
}

impl InstallationNotice {
    /// The notice to show, if any.
    pub(crate) fn view(&self) -> Option<InstallationNoticeView> {
        let status = self.status.as_ref()?;
        let actions = match &self.phase {
            Phase::Offer => vec![SECURE, COPY],
            Phase::Busy | Phase::Secured => Vec::new(),
            Phase::AskReplace => vec![REPLACE, KEEP],
            Phase::Copied { private: true, .. } => vec![RESTART, OPEN_ORIGINAL],
            Phase::Copied { private: false, .. } => vec![OPEN_ORIGINAL],
            Phase::PartlySecured => vec![COPY],
        };
        Some(InstallationNoticeView {
            body: body(status),
            detail: self.detail.clone(),
            busy: self.phase == Phase::Busy,
            actions,
        })
    }

    pub(crate) fn receive(&mut self, event: InstallationEvent) {
        match event {
            InstallationEvent::Checked(status) => {
                if self.status.is_none() {
                    self.status = status;
                    self.phase = Phase::Offer;
                }
            }
            _ if self.status.is_none() => {}
            InstallationEvent::Secured { status, failures } => {
                if status.is_private() && failures.is_empty() {
                    self.phase = Phase::Secured;
                    self.detail = format!(
                        "Secured: only you, administrators and the system can change {} now.",
                        shown(&status.folder)
                    );
                } else {
                    self.phase = Phase::PartlySecured;
                    self.detail = remaining(&status, &failures);
                }
            }
            InstallationEvent::SecureFailed(error) => {
                self.phase = Phase::Offer;
                self.detail = format!("The folder could not be secured: {error}");
            }
            InstallationEvent::Copied {
                folder,
                files,
                status,
                ..
            } => {
                let private = status.is_private();
                self.detail = if private {
                    format!(
                        "Copied and verified {files} files in {}. Restart from the copy; the \
                         original folder stays until you delete it.",
                        shown(&folder)
                    )
                } else {
                    format!(
                        "Copied and verified {files} files in {}, but other accounts can \
                         change that folder too, so Midnight Explorer does not restart from it.",
                        shown(&folder)
                    )
                };
                self.phase = Phase::Copied { folder, private };
            }
            InstallationEvent::CopyExists(folder) => {
                self.phase = Phase::AskReplace;
                self.detail = format!(
                    "{} already holds a copy of Midnight Explorer. Replace it?",
                    shown(&folder)
                );
            }
            InstallationEvent::CopyFailed(error) => {
                self.phase = Phase::Offer;
                self.detail = format!("Midnight Explorer could not be copied: {error}");
            }
            InstallationEvent::Restarting => self.close_requested.set(true),
            InstallationEvent::RestartFailed(error) => {
                self.phase = self.restart_target.take().unwrap_or_default();
                self.detail = format!("The copy could not be started: {error}");
            }
        }
    }

    fn begin(&mut self, detail: &str) {
        self.phase = Phase::Busy;
        self.detail = detail.into();
    }
}

impl Presenter {
    pub(crate) fn installation_notice_event(&mut self, event: InstallationEvent) {
        self.installation_notice.receive(event);
    }

    /// The notice's `index`-th button (before Dismiss).
    pub(crate) fn installation_notice_act(&mut self, index: usize, send: &dyn Fn(Command) -> bool) {
        let Some(view) = self.installation_notice.view() else {
            return;
        };
        let Some(&action) = view.actions.get(index) else {
            return;
        };
        let notice = &mut self.installation_notice;
        let (request, detail) = match action {
            SECURE => (InstallationRequest::Secure, "Securing the folder…"),
            COPY => (
                InstallationRequest::CopyToProfile { replace: false },
                "Copying Midnight Explorer into your profile and verifying the copy…",
            ),
            REPLACE => (
                InstallationRequest::CopyToProfile { replace: true },
                "Replacing the copy in your profile and verifying it…",
            ),
            KEEP => {
                notice.phase = Phase::Offer;
                notice.detail = "The existing copy was kept.".into();
                return;
            }
            RESTART => {
                notice.restart_target = Some(notice.phase.clone());
                (InstallationRequest::RestartFromCopy, "Starting the copy…")
            }
            OPEN_ORIGINAL => {
                let folder = notice.status.as_ref().map(|status| status.folder.clone());
                if let Some(folder) = folder {
                    let pane = self.active;
                    self.navigate(pane, folder, None, send);
                }
                return;
            }
            _ => return,
        };
        if send(Command::Installation(request)) {
            notice.begin(detail);
        } else {
            notice.detail = "Midnight Explorer is busy; try again.".into();
        }
    }

    /// Hide the notice and remember that for this installation folder.
    pub(crate) fn dismiss_installation_notice(&mut self, send: &dyn Fn(Command) -> bool) {
        if self.installation_notice.status.take().is_some() {
            send(Command::Installation(InstallationRequest::Dismiss));
        }
    }
}

#[cfg(test)]
pub(crate) fn status(folder: &str, shared_count: usize) -> InstallationStatus {
    InstallationStatus {
        folder: folder.into(),
        shared: (shared_count > 0)
            .then(|| folder.into())
            .into_iter()
            .collect(),
        shared_count,
        ..Default::default()
    }
}
