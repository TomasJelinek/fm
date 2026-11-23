//! The shared-installation notice above the panels, with the production callbacks: shown
//! from the backend's startup check, its buttons send their requests, the answers replace
//! its text, and Dismiss hides it and asks the backend to remember that.
#[path = "support/owned_commands.rs"]
mod owned_commands;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::items::AccessibleRole;
use midnight_core::{Command, Event, InstallationEvent, InstallationRequest, InstallationStatus};
use midnight_ui::{BeginOwned, InstallationNoticePresentation, MainWindow, Presenter};
use slint::ComponentHandle;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

struct Notice {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    begin: BeginOwned,
    commands: Rc<RefCell<Vec<Command>>>,
    folder: PathBuf,
}

impl Notice {
    fn open() -> Self {
        let mut ui = ui::TestUi::new();
        ui.resize(1200, 700);
        let base = std::env::temp_dir().join("installation notice");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            base.clone(),
            base.join("other"),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        let begin = owned_commands::install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        ui.draw();
        let folder = if cfg!(windows) {
            PathBuf::from(r"\\?\C:\tools\me")
        } else {
            PathBuf::from("/srv/tools/me")
        };
        Self {
            ui,
            presenter,
            begin,
            commands,
            folder,
        }
    }

    fn window(&self) -> &MainWindow {
        &self.ui.window
    }

    fn notice(&self) -> InstallationNoticePresentation<'_> {
        self.ui.window.global::<InstallationNoticePresentation>()
    }

    /// Deliver a backend answer the way the application's event loop does.
    fn answer(&mut self, event: InstallationEvent) {
        owned_commands::run(&self.presenter, &self.begin, |presenter, send| {
            presenter.apply_with_sender(Event::Installation(Box::new(event)), send)
        });
        midnight_ui::refresh(self.window(), &self.presenter.borrow());
        self.ui.draw();
    }

    fn press(&mut self, label: &str) {
        ui::click_root(self.window(), label, AccessibleRole::Button);
        self.ui.draw();
    }

    fn requests(&self) -> Vec<InstallationRequest> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Installation(request) => Some(*request),
                _ => None,
            })
            .collect()
    }

    fn status(&self, shared_count: usize) -> InstallationStatus {
        InstallationStatus {
            folder: self.folder.clone(),
            shared: vec![self.folder.clone()],
            shared_count,
            ..Default::default()
        }
    }
}

#[test]
fn the_notice_shows_once_routes_its_buttons_and_dismiss_is_remembered() {
    let mut notice = Notice::open();
    assert!(!notice.notice().get_visible());
    assert!(
        ui::try_root_item(
            notice.window(),
            "Secure this folder",
            AccessibleRole::Button
        )
        .is_none()
    );
    let status = notice.status(2828);
    notice.answer(InstallationEvent::Checked(Some(status)));
    assert!(notice.notice().get_visible());
    let body = notice.notice().get_body().to_string();
    assert!(body.contains("Other accounts on this"), "{body}");
    assert!(
        body.contains("copy Midnight Explorer into your profile"),
        "{body}"
    );
    assert!(
        !body.contains(r"\\?\"),
        "the shown folder has no verbatim prefix: {body}"
    );
    render::snapshot(&notice.ui, "installation-notice");
    notice.press("Secure this folder");
    assert_eq!(notice.requests(), [InstallationRequest::Secure]);
    assert!(notice.notice().get_busy());
    let secured = InstallationStatus {
        folder: notice.folder.clone(),
        ..Default::default()
    };
    notice.answer(InstallationEvent::Secured {
        status: secured,
        failures: Vec::new(),
    });
    let detail = notice.notice().get_detail().to_string();
    assert!(detail.starts_with("Secured"), "{detail}");
    render::snapshot(&notice.ui, "installation-secured");
    notice.press("Dismiss");
    assert!(!notice.notice().get_visible());
    assert_eq!(
        notice.requests(),
        [InstallationRequest::Secure, InstallationRequest::Dismiss]
    );
}

#[test]
fn moving_to_the_profile_offers_the_restart_from_a_verified_private_copy() {
    let mut notice = Notice::open();
    let status = notice.status(2828);
    notice.answer(InstallationEvent::Checked(Some(status)));
    notice.press("Move to my profile…");
    assert_eq!(
        notice.requests(),
        [InstallationRequest::CopyToProfile { replace: false }]
    );
    let copy = if cfg!(windows) {
        PathBuf::from(r"C:\Users\me\AppData\Local\Programs\Midnight Explorer")
    } else {
        PathBuf::from("/home/me/.local/share/midnight-explorer/app")
    };
    notice.answer(InstallationEvent::CopyExists(copy.clone()));
    render::snapshot(&notice.ui, "installation-copy-exists");
    notice.press("Replace the copy");
    notice.answer(InstallationEvent::Copied {
        folder: copy.clone(),
        files: 2827,
        bytes: 1,
        status: InstallationStatus {
            folder: copy,
            ..Default::default()
        },
    });
    assert!(notice.notice().get_detail().contains("2827 files"));
    render::snapshot(&notice.ui, "installation-copied");
    notice.press("Restart from the copy");
    assert_eq!(
        notice.requests().last(),
        Some(&InstallationRequest::RestartFromCopy)
    );
}
