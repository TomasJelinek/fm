use super::*;
use crate::presenter::installation_notice::status;
use midnight_core::{InstallationEvent, InstallationRequest};
use std::cell::RefCell;

fn shared(p: &mut Presenter) -> PathBuf {
    let folder = test_root().join("tools").join("me");
    let mut status = status(folder.to_str().unwrap(), 4);
    status.shared_ancestor = Some(test_root().join("tools"));
    p.apply(Event::Installation(Box::new(InstallationEvent::Checked(
        Some(status),
    ))));
    folder
}

fn requests(sent: &RefCell<Vec<Command>>) -> Vec<InstallationRequest> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            Command::Installation(request) => Some(*request),
            _ => None,
        })
        .collect()
}

#[test]
fn a_private_or_dismissed_installation_shows_no_notice() {
    let mut p = Presenter::new(test_root(), "right".into());
    assert!(p.installation_notice.view().is_none());
    p.apply(Event::Installation(Box::new(InstallationEvent::Checked(
        None,
    ))));
    assert!(p.installation_notice.view().is_none());
}

#[test]
fn the_notice_offers_securing_and_copying_and_dismiss_is_remembered_once() {
    let mut p = Presenter::new(test_root(), "right".into());
    let folder = shared(&mut p);
    let view = p.installation_notice.view().expect("shown");
    assert!(
        view.body.contains(&folder.display().to_string()),
        "{}",
        view.body
    );
    assert!(
        view.body.starts_with("Other accounts on this"),
        "{}",
        view.body
    );
    assert!(
        view.body
            .contains("copy Midnight Explorer into your profile")
    );
    for jargon in [
        "deploy-private",
        "RegisterRuntime",
        "launcher pointer",
        "move it",
    ] {
        assert!(!view.body.contains(jargon), "{}", view.body);
    }
    assert_eq!(view.actions, ["Secure this folder", "Move to my profile…"]);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.dismiss_installation_notice(&send);
    assert!(p.installation_notice.view().is_none());
    assert_eq!(requests(&sent), [InstallationRequest::Dismiss]);
    // A second dismissal and a later startup answer do not show or remember it again.
    p.dismiss_installation_notice(&send);
    p.apply(Event::Installation(Box::new(InstallationEvent::Secured {
        status: status("x", 0),
        failures: Vec::new(),
    })));
    assert!(p.installation_notice.view().is_none());
    assert_eq!(requests(&sent).len(), 1);
}

#[test]
fn securing_reports_success_or_what_other_accounts_can_still_change() {
    let mut p = Presenter::new(test_root(), "right".into());
    let folder = shared(&mut p);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.installation_notice_act(0, &send);
    assert_eq!(requests(&sent), [InstallationRequest::Secure]);
    let busy = p.installation_notice.view().unwrap();
    assert!(busy.busy && busy.actions.is_empty(), "{busy:?}");
    let mut remaining = status(folder.to_str().unwrap(), 0);
    remaining.shared_ancestor = Some(test_root().join("tools"));
    p.apply(Event::Installation(Box::new(InstallationEvent::Secured {
        status: remaining,
        failures: vec!["LICENSE.txt: Access is denied.".into()],
    })));
    let view = p.installation_notice.view().unwrap();
    assert!(view.detail.contains("rename or replace"), "{}", view.detail);
    assert!(view.detail.contains("LICENSE.txt"), "{}", view.detail);
    assert_eq!(view.actions, ["Move to my profile…"]);
    p.apply(Event::Installation(Box::new(InstallationEvent::Secured {
        status: status(folder.to_str().unwrap(), 0),
        failures: Vec::new(),
    })));
    let view = p.installation_notice.view().unwrap();
    assert!(view.detail.starts_with("Secured"), "{}", view.detail);
    assert!(view.actions.is_empty());
    p.apply(Event::Installation(Box::new(
        InstallationEvent::SecureFailed("belongs to another account".into()),
    )));
    let view = p.installation_notice.view().unwrap();
    assert!(view.detail.contains("belongs to another account"));
    assert_eq!(view.actions, ["Secure this folder", "Move to my profile…"]);
}

#[test]
fn copying_asks_before_replacing_then_offers_restart_and_the_original_folder() {
    let mut p = Presenter::new(test_root(), "right".into());
    let folder = shared(&mut p);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.installation_notice_act(1, &send);
    let copy = test_root().join("profile").join("Midnight Explorer");
    p.apply(Event::Installation(Box::new(
        InstallationEvent::CopyExists(copy.clone()),
    )));
    let view = p.installation_notice.view().unwrap();
    assert_eq!(view.actions, ["Replace the copy", "Keep the existing copy"]);
    p.installation_notice_act(0, &send);
    assert_eq!(
        requests(&sent),
        [
            InstallationRequest::CopyToProfile { replace: false },
            InstallationRequest::CopyToProfile { replace: true }
        ]
    );
    p.apply(Event::Installation(Box::new(InstallationEvent::Copied {
        folder: copy.clone(),
        files: 2811,
        bytes: 1,
        status: status(copy.to_str().unwrap(), 0),
    })));
    let view = p.installation_notice.view().unwrap();
    assert!(view.detail.contains("2811 files"), "{}", view.detail);
    assert_eq!(
        view.actions,
        ["Restart from the copy", "Open the original folder"]
    );
    // Opening the original folder navigates the active panel there.
    p.installation_notice_act(1, &send);
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Navigate { path, .. } if *path == folder))
    );
    p.installation_notice_act(0, &send);
    assert_eq!(
        requests(&sent).last(),
        Some(&InstallationRequest::RestartFromCopy)
    );
    p.apply(Event::Installation(Box::new(
        InstallationEvent::RestartFailed("missing".into()),
    )));
    let view = p.installation_notice.view().unwrap();
    assert_eq!(
        view.actions,
        ["Restart from the copy", "Open the original folder"]
    );
    assert!(!p.installation_notice.close_requested.get());
    p.installation_notice_act(0, &send);
    p.apply(Event::Installation(Box::new(InstallationEvent::Restarting)));
    assert!(p.installation_notice.close_requested.get());
    // A copy other accounts can change is never offered for a restart.
    let mut p = Presenter::new(test_root(), "right".into());
    shared(&mut p);
    p.apply(Event::Installation(Box::new(InstallationEvent::Copied {
        folder: copy.clone(),
        files: 1,
        bytes: 1,
        status: status(copy.to_str().unwrap(), 1),
    })));
    assert_eq!(
        p.installation_notice.view().unwrap().actions,
        ["Open the original folder"]
    );
}

#[test]
fn a_refused_request_keeps_the_offer() {
    let mut p = Presenter::new(test_root(), "right".into());
    shared(&mut p);
    p.installation_notice_act(0, &|_| false);
    let view = p.installation_notice.view().unwrap();
    assert!(!view.busy);
    assert_eq!(view.actions, ["Secure this folder", "Move to my profile…"]);
}
