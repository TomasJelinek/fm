//! Chromium's sandbox refused to the HTML viewer (Linux: AppArmor restricts unprivileged user
//! namespaces, or the kernel disables them). The viewer shows why instead of the page and,
//! where an administrator can install the permission from here, offers Install…: its
//! authorization and installation run on their own thread, and the document opens again
//! once the permission is installed. Nothing is checked or installed before a browser was
//! refused.
use super::*;
use midnight_elevation::{SandboxProfile, SandboxProfileError};
use midnight_web_viewer_host::SandboxRefusal;
use std::{
    path::Path,
    sync::{Mutex, PoisonError},
};

#[cfg(test)]
#[path = "worker_sandbox_tests.rs"]
mod tests;

/// Installs the permission. Blocks until the administrator answered and the installation
/// ended; a cancelled token stops a pending authorization.
pub(super) type Install =
    Arc<dyn Fn(&Cancellation) -> Result<(), SandboxProfileError> + Send + Sync>;
/// The installation that an administrator can authorize from here for a refused browser
/// executable, if any. It inspects the file system only.
pub(super) type Offer = Arc<dyn Fn(&Path) -> Option<Offered> + Send + Sync>;

/// Install… for a refused browser: the installation and what it allows, which the notice
/// states before anyone is asked.
#[derive(Clone)]
pub(super) struct Offered {
    pub(super) install: Install,
    /// The viewer executable the permission is for, or for a build in a source checkout
    /// the pattern that names every build there.
    pub(super) viewer: String,
    /// Someone other than root, such as any program the user runs, can change a folder on
    /// the way to `viewer`, and could put another program there.
    pub(super) changeable: bool,
}

const WAITING: &str = "Waiting for authorization…";
const DECLINED: &str = "Authorization was cancelled or refused, so nothing was installed. \
                        Install… asks again.";
const NO_AGENT: &str = "No authentication agent runs in this session, so nobody can be asked \
                        to authorize it from here.";
const STILL_REFUSED: &str = "Its AppArmor profile was installed from this viewer, but the system \
                             still refuses the sandbox (see system-requirements.md).";

/// The platform's offer: the HTML viewer's AppArmor profile on Linux, nothing elsewhere.
pub(super) fn system_offer() -> Offer {
    Arc::new(|executable: &Path| {
        SandboxProfile::for_browser(executable).map(|profile| Offered {
            viewer: profile.attachment().to_owned(),
            changeable: profile.changeable(),
            install: Arc::new(move |cancel: &Cancellation| profile.install(cancel)),
        })
    })
}

/// What Install… does, for whom, and what else it allows where the viewer's folder can be
/// changed without root.
fn offer_text(offered: &Offered) -> String {
    let mut text = format!(
        "Install… asks an administrator to let the HTML viewer at {} use user namespaces, \
         with the AppArmor profile midnight-explorer-html-viewer, which one installation at \
         a time can have.",
        offered.viewer
    );
    if offered.changeable {
        text.push_str(
            " Programs running as you can change that folder, so a program put at that path \
             could use user namespaces too.",
        );
    }
    text
}

/// One viewer window's state of the permission.
pub(super) struct Sandbox {
    offer: Offer,
    /// The current document's browser was refused; cleared when a document opens.
    refused: Option<Refused>,
    /// The installation running on its own thread, which stores its result here and wakes
    /// the worker.
    installing: Option<Installing>,
    /// An installation succeeded in this window, so a later refusal is not offered again.
    installed: bool,
    /// A notice is shown, which the next document that opens removes.
    shown: bool,
}

impl Sandbox {
    pub(super) fn new(offer: Offer) -> Self {
        Self {
            offer,
            refused: None,
            installing: None,
            installed: false,
            shown: false,
        }
    }
}

struct Refused {
    /// The browser host's explanation, with the manual step that allows the sandbox.
    message: String,
    /// What the system refuses, without that step.
    cause: &'static str,
    offered: Option<Offered>,
    /// Why Install… is no longer offered, where it was and the reason is worth saying.
    withdrawn: Option<&'static str>,
    /// An AppArmor profile allows what was refused.
    profile_allows: bool,
}

struct Installing {
    result: Arc<Mutex<Option<Result<(), SandboxProfileError>>>>,
    /// Observes the window's close flag.
    cancel: Cancellation,
}

impl Drop for Installing {
    fn drop(&mut self) {
        // A pending authorization ends with the window that asked for it. The thread then
        // ends by itself; nothing waits for it here.
        self.cancel.cancel();
    }
}

impl Worker {
    /// The browser host stopped the current document's browser because the system refused
    /// its sandbox. Show why instead of the page, and offer the installation where an
    /// administrator can authorize it from here and it would help.
    pub(super) fn sandbox_refused(
        &mut self,
        refusal: SandboxRefusal,
        message: String,
        output: &Mailbox,
    ) {
        let profile_allows = matches!(refusal, SandboxRefusal::UserNamespaces { .. });
        let offered = match &refusal {
            SandboxRefusal::UserNamespaces { executable } if !self.sandbox.installed => {
                (self.sandbox.offer)(executable)
            }
            // The kernel's own switch: no profile could allow it.
            _ => None,
        };
        self.invalidate_layout();
        output.finish_open(Err(message.clone()));
        self.sandbox.refused = Some(Refused {
            message,
            cause: refusal.cause(),
            offered,
            withdrawn: None,
            profile_allows,
        });
        self.show_refusal(output);
    }

    /// Install…: start the offered installation on its own thread, once at a time.
    pub(super) fn install_sandbox_permission(&mut self, output: &Mailbox) {
        if self.sandbox.installing.is_some() {
            return;
        }
        let Some(install) = self
            .sandbox
            .refused
            .as_ref()
            .and_then(|refused| refused.offered.as_ref())
            .map(|offered| offered.install.clone())
        else {
            return;
        };
        let result = Arc::new(Mutex::new(None));
        let cancel = Cancellation::observing(self.window_closed.clone());
        let (slot, wake, token) = (result.clone(), self.wake.clone(), cancel.clone());
        let started = std::thread::Builder::new()
            .name("viewer-sandbox-permission".into())
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install(&token)))
                        .unwrap_or_else(|_| {
                            Err(SandboxProfileError::Failed(
                                "The installation stopped unexpectedly.".into(),
                            ))
                        });
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
                wake();
            });
        match started {
            Ok(_) => {
                self.sandbox.installing = Some(Installing { result, cancel });
                self.show_refusal(output);
            }
            Err(error) => self.show_outcome(
                &format!(
                    "The profile could not be installed: cannot start the installation: {error}"
                ),
                output,
            ),
        }
    }

    /// A finished installation: open the document again once it worked, else say why.
    pub(super) fn poll_sandbox(&mut self, output: &Mailbox) {
        let Some(result) = self.sandbox.installing.as_ref().and_then(|installing| {
            installing
                .result
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        }) else {
            return;
        };
        self.sandbox.installing = None;
        match result {
            Ok(()) => {
                self.sandbox.installed = true;
                // The profile applies to the next browser that starts.
                if self.sandbox.refused.is_some() {
                    self.open(output);
                }
            }
            Err(SandboxProfileError::Declined) => self.show_outcome(DECLINED, output),
            // Only the manual instructions remain.
            Err(SandboxProfileError::Unavailable) => self.withdraw_offer(None, output),
            // Asking again could never work: say why instead of offering it.
            Err(SandboxProfileError::NoAgent) => self.withdraw_offer(Some(NO_AGENT), output),
            Err(SandboxProfileError::Failed(error)) => self.show_outcome(
                &format!("The profile could not be installed: {error}"),
                output,
            ),
        }
    }

    /// A document opens: whatever the previous one showed instead of its page goes.
    pub(super) fn clear_sandbox_notice(&mut self, output: &Mailbox) {
        self.sandbox.refused = None;
        if std::mem::take(&mut self.sandbox.shown) {
            output.put(ViewerOutput::SandboxNotice(None));
        }
    }

    /// Install… is not offered any more for the current refusal, for `reason` if given.
    fn withdraw_offer(&mut self, reason: Option<&'static str>, output: &Mailbox) {
        if let Some(refused) = &mut self.sandbox.refused {
            refused.offered = None;
            refused.withdrawn = reason;
        }
        self.show_refusal(output);
    }

    /// The current refusal: the offer, the pending authorization, or the manual step.
    fn show_refusal(&mut self, output: &Mailbox) {
        let Some(refused) = &self.sandbox.refused else {
            return;
        };
        let installable = refused.offered.is_some();
        let installing = installable && self.sandbox.installing.is_some();
        let message = if installing {
            format!("{} {WAITING}", refused.cause)
        } else if let Some(offered) = &refused.offered {
            format!("{} {}", refused.cause, offer_text(offered))
        } else if self.sandbox.installed && refused.profile_allows {
            format!("{} {STILL_REFUSED}", refused.cause)
        } else if let Some(reason) = refused.withdrawn {
            format!("{} {reason}", refused.message)
        } else {
            refused.message.clone()
        };
        self.show_notice(
            ViewerSandboxNotice {
                message,
                installable,
                installing,
                details: None,
            },
            output,
        );
    }

    /// How an installation for the current refusal ended, with Install… still offered.
    fn show_outcome(&mut self, outcome: &str, output: &Mailbox) {
        let Some(refused) = &self.sandbox.refused else {
            // Another document is shown by now.
            return;
        };
        let notice = ViewerSandboxNotice {
            message: format!("{} {outcome}", refused.cause),
            installable: refused.offered.is_some(),
            installing: false,
            details: None,
        };
        self.show_notice(notice, output);
    }

    /// A document that failed with diagnostic details: its message in the page's place with
    /// Copy details, until another document opens. The caller reports the failure on the
    /// status line (`Worker::error`), as for one without details, which shows no notice.
    pub(super) fn show_failure_notice(
        &mut self,
        message: String,
        details: String,
        output: &Mailbox,
    ) {
        self.sandbox.shown = true;
        output.put(ViewerOutput::SandboxNotice(Some(ViewerSandboxNotice {
            message,
            installable: false,
            installing: false,
            details: Some(details),
        })));
    }

    fn show_notice(&mut self, notice: ViewerSandboxNotice, output: &Mailbox) {
        self.sandbox.shown = true;
        self.status(notice.message.clone(), output);
        output.put(ViewerOutput::SandboxNotice(Some(notice)));
    }
}
