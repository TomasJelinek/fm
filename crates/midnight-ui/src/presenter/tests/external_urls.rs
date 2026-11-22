//! Links that viewers hand to the system's default handler through the presenter.
use super::*;
use std::rc::Rc;

/// What a link's `done` callback heard.
type Outcomes = Rc<RefCell<Vec<Result<String, String>>>>;

fn presenter() -> Presenter {
    let root = test_root();
    Presenter::new(root.join("left"), root.join("right"))
}

/// Hands `url` over with `send` and records what its callback hears.
fn open_with(p: &mut Presenter, url: &ExternalUrl, send: &dyn Fn(Command) -> bool) -> Outcomes {
    let outcomes = Outcomes::default();
    let heard = outcomes.clone();
    p.open_external_url(
        url.clone(),
        send,
        Box::new(move |outcome| heard.borrow_mut().push(outcome)),
    );
    outcomes
}

/// Queues `url`, checks that only its typed request went out, and returns that request.
fn open(p: &mut Presenter, url: &ExternalUrl) -> (DesktopRequestId, Outcomes) {
    let sent = RefCell::new(Vec::new());
    let outcomes = open_with(p, url, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let request = match sent.into_inner().as_slice() {
        [
            Command::OpenUrl {
                request,
                url: queued,
            },
        ] if queued == url => *request,
        commands => panic!("only the typed link request is sent: {commands:?}"),
    };
    assert!(
        outcomes.borrow().is_empty(),
        "nothing is reported before the backend answers"
    );
    (request, outcomes)
}

fn answer(
    p: &mut Presenter,
    request: DesktopRequestId,
    result: Result<DesktopActionResult, FsError>,
) {
    p.apply(Event::DesktopActionFinished { request, result });
}

fn refused(message: &str) -> FsError {
    FsError::from_io(
        "open link",
        Path::new(""),
        io::Error::new(io::ErrorKind::NotFound, message.to_owned()),
    )
}

#[test]
fn a_mail_link_without_its_attachments_says_so_once_the_mail_program_took_it() {
    let mut p = presenter();
    let status = p.desktop.status.clone();
    let url =
        ExternalUrl::parse("mailto:someone@example.com?subject=Hi&attach=%2Fetc%2Fpasswd").unwrap();
    assert!(url.attachments_removed());
    let (request, outcomes) = open(&mut p, &url);
    // Another request's answer is not this link's.
    answer(
        &mut p,
        DesktopRequestId(request.0 + 1),
        Ok(DesktopActionResult::UrlOpened { mail: true }),
    );
    assert!(outcomes.borrow().is_empty());
    answer(
        &mut p,
        request,
        Ok(DesktopActionResult::UrlOpened { mail: true }),
    );
    assert_eq!(
        *outcomes.borrow(),
        [Ok(
            "Opened in your mail program: mailto:someone@example.com?subject=Hi \
             (without the file attachments it asked for)"
                .to_owned()
        )]
    );
    // Heard once; the main window's status line and notices belong to the panels.
    answer(
        &mut p,
        request,
        Ok(DesktopActionResult::UrlOpened { mail: true }),
    );
    assert_eq!(outcomes.borrow().len(), 1);
    assert_eq!(p.desktop.status, status);
    assert!(p.desktop.notice.is_none());
}

#[test]
fn web_links_open_in_the_browser_and_plain_mail_links_carry_no_note() {
    let mut p = presenter();
    for (link, expected) in [
        (
            "https://example.com/a b?q=\"x\"",
            "Opened in your browser: https://example.com/a%20b?q=%22x%22",
        ),
        (
            "mailto:someone@example.com?subject=Hi",
            "Opened in your mail program: mailto:someone@example.com?subject=Hi",
        ),
    ] {
        let url = ExternalUrl::parse(link).unwrap();
        let (request, outcomes) = open(&mut p, &url);
        answer(
            &mut p,
            request,
            Ok(DesktopActionResult::UrlOpened {
                mail: url.is_mail(),
            }),
        );
        assert_eq!(*outcomes.borrow(), [Ok(expected.to_owned())]);
    }
}

#[test]
fn a_refused_link_names_the_reason_and_never_becomes_a_main_window_notice() {
    let mut p = presenter();
    let url = ExternalUrl::parse("https://example.com/report").unwrap();
    let (request, outcomes) = open(&mut p, &url);
    answer(
        &mut p,
        request,
        Err(refused(
            "No program opens\u{7}this kind of link.\nError kind: NotFound",
        )),
    );
    assert_eq!(
        *outcomes.borrow(),
        [Err(
            "Could not open https://example.com/report: No program opens\u{fffd}this kind of link."
                .to_owned()
        )]
    );
    assert!(p.desktop.notice.is_none());
}

#[test]
fn a_link_that_cannot_be_queued_fails_at_once_with_the_busy_reason() {
    let mut p = presenter();
    let url = ExternalUrl::parse("https://example.com/").unwrap();
    let outcomes = open_with(&mut p, &url, &|command| {
        assert!(matches!(command, Command::OpenUrl { .. }));
        false
    });
    assert_eq!(
        *outcomes.borrow(),
        [Err(
            "Could not open https://example.com/: Desktop queue busy. Try opening the link again."
                .to_owned()
        )]
    );
    assert!(p.desktop.notice.is_none());
}

#[test]
fn long_addresses_are_shortened_so_the_outcome_and_its_reason_stay_readable() {
    let mut p = presenter();
    let long = "a".repeat(5000);
    let url = ExternalUrl::parse(&format!(
        "mailto:someone@example.com?subject={long}&attach=x"
    ))
    .unwrap();
    let (request, outcomes) = open(&mut p, &url);
    answer(
        &mut p,
        request,
        Ok(DesktopActionResult::UrlOpened { mail: true }),
    );
    let opened = outcomes.borrow_mut().remove(0).unwrap();
    assert_eq!(opened.chars().count(), 300, "{opened}");
    assert!(
        opened.starts_with("Opened in your mail program: mailto:someone@example.com?subject=aaa"),
        "{opened}"
    );
    assert!(
        opened.ends_with("a… (without the file attachments it asked for)"),
        "{opened}"
    );

    let url = ExternalUrl::parse(&format!("https://example.com/{long}")).unwrap();
    let (request, outcomes) = open(&mut p, &url);
    answer(
        &mut p,
        request,
        Err(refused("Access is denied to photo\u{202e}gpj.exe")),
    );
    let failed = outcomes.borrow_mut().remove(0).unwrap_err();
    assert_eq!(failed.chars().count(), 300, "{failed}");
    assert!(
        failed.starts_with("Could not open https://example.com/aaa"),
        "{failed}"
    );
    assert!(
        failed.ends_with("a…: Access is denied to photo\\u202Egpj.exe"),
        "{failed}"
    );
    assert!(!failed.contains('\u{202e}'), "{failed}");
}

/// A link's folder opens in the active panel; another file is shown in its folder with
/// the cursor on it. Neither is opened by a program.
#[test]
fn local_link_targets_are_shown_in_the_active_panel_with_the_cursor_on_a_file() {
    let mut p = presenter();
    p.active = PaneId::Right;
    let root = test_root();
    let folder = root.join("docs");
    let sent = RefCell::new(Vec::new());
    let send = |command: Command| {
        sent.borrow_mut().push(command);
        true
    };
    let shown = p.show_link_target(&folder, true, &send);
    assert!(
        shown.starts_with("Opened ") && shown.ends_with(" in the active panel."),
        "{shown}"
    );
    assert!(matches!(
        &sent.borrow()[..],
        [Command::Navigate { pane: PaneId::Right, path, .. }] if *path == folder
    ));
    assert_eq!(p.panes[1].pending.as_ref().unwrap().wanted_cursor, None);
    sent.borrow_mut().clear();
    let program = folder.join("tool.exe");
    let shown = p.show_link_target(&program, false, &send);
    assert!(shown.starts_with("No viewer shows "), "{shown}");
    assert!(matches!(
        &sent.borrow()[..],
        [Command::Navigate { pane: PaneId::Right, path, .. }] if *path == folder
    ));
    assert_eq!(
        p.panes[1].pending.as_ref().unwrap().wanted_cursor.as_ref(),
        Some(&program)
    );
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::OpenEntry { .. } | Command::OpenUrl { .. })),
        "nothing is opened"
    );
    let busy = p.show_link_target(&folder, true, &|_| false);
    assert_eq!(busy, "The panel is busy. Follow the link again.");
}
