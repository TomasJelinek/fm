//! The notice and Install… of a refused browser sandbox (`worker_sandbox.rs`), with a
//! scripted browser session and a scripted installation instead of the system's.
use super::*;
use crate::worker::web::{Browser, WebView};
use midnight_core::viewer_web as api;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

const EXECUTABLE: &str = "/opt/me/viewers/midnight-viewer-html/midnight-viewer-html";
const REFUSED: &str = "The HTML viewer needs permission for Chromium's sandbox on this system \
                       (Ubuntu restricts unprivileged user namespaces). Run once: sudo \
                       /opt/me/install-html-viewer-apparmor.sh";
const WAIT: Duration = Duration::from_secs(10);

fn refusal() -> SandboxRefusal {
    SandboxRefusal::UserNamespaces {
        executable: EXECUTABLE.into(),
    }
}

fn cause() -> &'static str {
    refusal().cause()
}

/// A browser session that its host stopped: it drains its output once and reports
/// `refusal` as the reason of its terminal error.
struct Ended {
    events: Mutex<Vec<api::Event>>,
    refusal: Option<SandboxRefusal>,
}

impl Browser for Ended {
    fn send(&self, _: api::CommandKind) -> bool {
        false
    }
    fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let mut events = self.events.lock().unwrap();
        let count = maximum.min(events.len());
        events.drain(..count).collect()
    }
    fn close(&self) {}
    fn finished(&self) -> bool {
        true
    }
    fn policy_change_allowed(&self) -> bool {
        true
    }
    fn release_on_worker(self: Box<Self>) -> Result<(), String> {
        Ok(())
    }
    fn sandbox_refusal(&self) -> Option<SandboxRefusal> {
        self.refusal.clone()
    }
}

/// Show an HTML document whose browser ended with the terminal error `REFUSED`, which the
/// host reports as `refusal` when given.
fn show(worker: &mut Worker, output: &Mailbox, refusal: Option<SandboxRefusal>) {
    let viewport = api::Viewport {
        revision: 1,
        width: worker.geometry.width,
        height: worker.geometry.height,
        scale: 1.0,
    };
    let error = api::Event {
        header: api::Header {
            session: 1,
            document: 1,
            sequence: 1,
        },
        kind: api::EventKind::Error {
            code: api::ErrorCode::Runtime,
            message: REFUSED.into(),
        },
    };
    let session = Box::new(Ended {
        events: Mutex::new(vec![error]),
        refusal,
    });
    worker.web = Some(WebView::new(session, viewport, 1));
    worker.poll_web(output);
}

/// A worker whose `offer` stands for the system's, showing a document of one file whose
/// browser `refusal` stopped.
fn refused(refusal: Option<SandboxRefusal>, offer: Offer) -> (Worker, Mailbox) {
    let (mut worker, output) = crate::worker::tests::worker();
    worker.sandbox = Sandbox::new(offer);
    worker.files = Arc::from([PathBuf::from("/srv/page.html")]);
    show(&mut worker, &output, refusal);
    (worker, output)
}

type Outcome = Result<(), SandboxProfileError>;

/// The test's side of an offered installation: each start sends its cancellation token on
/// `starts` and returns the next outcome sent to `finish`, or a failure once cancelled.
struct Scripted {
    offer: Offer,
    offers: Arc<AtomicUsize>,
    starts: mpsc::Receiver<Cancellation>,
    finish: mpsc::Sender<Outcome>,
}

fn scripted() -> Scripted {
    scripted_for(false)
}

/// `scripted`, for a viewer whose folder someone other than root can change when
/// `changeable`.
fn scripted_for(changeable: bool) -> Scripted {
    let (started, starts) = mpsc::channel();
    let (finish, outcomes) = mpsc::channel::<Outcome>();
    let outcomes = Arc::new(Mutex::new(outcomes));
    let offers = Arc::new(AtomicUsize::new(0));
    let counted = offers.clone();
    let offer: Offer = Arc::new(move |executable: &Path| {
        assert_eq!(executable, Path::new(EXECUTABLE));
        counted.fetch_add(1, Ordering::SeqCst);
        let (started, outcomes) = (started.clone(), outcomes.clone());
        let install: Install = Arc::new(move |cancel: &Cancellation| {
            started.send(cancel.clone()).unwrap();
            let outcomes = outcomes.lock().unwrap();
            loop {
                match outcomes.recv_timeout(Duration::from_millis(5)) {
                    Ok(outcome) => return outcome,
                    Err(mpsc::RecvTimeoutError::Timeout) if !cancel.is_cancelled() => {}
                    Err(_) => return Err(SandboxProfileError::Failed("cancelled".into())),
                }
            }
        });
        Some(Offered {
            install,
            viewer: EXECUTABLE.into(),
            changeable,
        })
    });
    Scripted {
        offer,
        offers,
        starts,
        finish,
    }
}

/// The warning for a viewer in a folder that someone other than root can change.
const CHANGEABLE: &str = "Programs running as you can change that folder";

/// Wakes of the worker, which a finished installation sends.
fn wakes(worker: &mut Worker) -> mpsc::Receiver<()> {
    let (woke, wakes) = mpsc::channel();
    worker.wake = Arc::new(move || woke.send(()).is_ok());
    wakes
}

fn notices(outputs: &[ViewerOutput]) -> Vec<Option<ViewerSandboxNotice>> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ViewerOutput::SandboxNotice(notice) => Some(notice.clone()),
            _ => None,
        })
        .collect()
}

fn notice(message: String, installable: bool, installing: bool) -> Option<ViewerSandboxNotice> {
    Some(ViewerSandboxNotice {
        message,
        installable,
        installing,
    })
}

/// Install…, until the scripted installation started; returns its cancellation token.
fn install(worker: &mut Worker, output: &Mailbox, script: &Scripted) -> Cancellation {
    worker.input(ViewerInput::InstallSandboxPermission, output);
    script
        .starts
        .recv_timeout(WAIT)
        .expect("the installation starts")
}

/// Finish the running installation with `outcome` and handle its wake.
fn finish(
    worker: &mut Worker,
    output: &Mailbox,
    script: &Scripted,
    wakes: &mpsc::Receiver<()>,
    outcome: Outcome,
) {
    script.finish.send(outcome).unwrap();
    wakes
        .recv_timeout(WAIT)
        .expect("the finished installation wakes the worker");
    worker.poll_sandbox(output);
}

/// The one notice in `outputs`, which must offer Install… (`installable`) as given.
fn single_notice(outputs: &[ViewerOutput], installable: bool) -> String {
    let [Some(notice)] = &notices(outputs)[..] else {
        panic!("one notice: {outputs:?}");
    };
    assert_eq!(notice.installable, installable, "{notice:?}");
    assert!(!notice.installing, "{notice:?}");
    notice.message.clone()
}

#[test]
fn a_refused_browser_offers_install_instead_of_the_page() {
    let script = scripted();
    let (worker, output) = refused(Some(refusal()), script.offer.clone());
    assert_eq!(output.take_open_result(), Some(Err(REFUSED.into())));
    let offered = single_notice(&output.take(), true);
    // The cause, then what Install… lets which viewer do, before anyone is asked.
    assert!(offered.starts_with(cause()), "{offered}");
    assert!(
        offered.contains(&format!(
            "the HTML viewer at {EXECUTABLE} use user namespaces"
        )),
        "{offered}"
    );
    assert!(!offered.contains(CHANGEABLE), "{offered}");
    assert_eq!(worker.status, offered);
    assert_eq!(script.offers.load(Ordering::SeqCst), 1);
}

#[test]
fn an_offer_for_a_folder_others_than_root_can_change_says_what_else_it_allows() {
    let script = scripted_for(true);
    let (_worker, output) = refused(Some(refusal()), script.offer.clone());
    let offered = single_notice(&output.take(), true);
    assert!(
        offered.contains(&format!("the HTML viewer at {EXECUTABLE}"))
            && offered.contains(CHANGEABLE)
            && offered.ends_with("could use user namespaces too."),
        "{offered}"
    );
}

#[test]
fn namespaces_the_kernel_disables_offer_no_install_and_name_the_block() {
    let script = scripted();
    let disabled = SandboxRefusal::NamespacesDisabled {
        setting: "kernel.unprivileged_userns_clone",
    };
    let (mut worker, output) = refused(Some(disabled), script.offer.clone());
    // The host's message, which names the setting, as it is: no profile could help.
    assert_eq!(single_notice(&output.take(), false), REFUSED);
    assert_eq!(
        script.offers.load(Ordering::SeqCst),
        0,
        "nothing is checked"
    );
    worker.input(ViewerInput::InstallSandboxPermission, &output);
    assert!(
        script
            .starts
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
}

#[test]
fn without_authorization_the_manual_step_is_shown_without_install() {
    let (worker, output) = refused(Some(refusal()), Arc::new(|_: &Path| None));
    assert_eq!(
        notices(&output.take()),
        [notice(REFUSED.into(), false, false)]
    );
    assert_eq!(worker.status, REFUSED);
}

#[test]
fn other_browser_errors_neither_offer_nor_check_anything() {
    let script = scripted();
    let (worker, output) = refused(None, script.offer.clone());
    assert!(notices(&output.take()).is_empty());
    assert_eq!(worker.status, REFUSED);
    assert_eq!(script.offers.load(Ordering::SeqCst), 0);
}

#[test]
fn install_runs_once_off_the_worker_and_success_opens_the_document_again() {
    let script = scripted();
    let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
    let wakes = wakes(&mut worker);
    output.take();
    let cancel = install(&mut worker, &output, &script);
    // A second request while the first waits for authorization starts nothing.
    worker.input(ViewerInput::InstallSandboxPermission, &output);
    assert!(
        script
            .starts
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    let waiting = format!("{} {WAITING}", cause());
    assert_eq!(
        notices(&output.take()),
        [notice(waiting.clone(), true, true)]
    );
    assert_eq!(worker.status, waiting);
    assert!(!cancel.is_cancelled());
    let generation = worker.generation;
    finish(&mut worker, &output, &script, &wakes, Ok(()));
    assert_eq!(
        notices(&output.take()),
        [None],
        "the page's place is free again"
    );
    assert_eq!(
        worker.generation,
        generation + 1,
        "the document opened again"
    );
    assert!(worker.sandbox.installing.is_none());
}

#[test]
fn declined_authorization_or_a_failure_keeps_install_available() {
    for (outcome, said) in [
        (SandboxProfileError::Declined, DECLINED.to_owned()),
        (
            SandboxProfileError::Failed("AppArmor refused the profile.".into()),
            "The profile could not be installed: AppArmor refused the profile.".to_owned(),
        ),
    ] {
        let script = scripted();
        let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
        let wakes = wakes(&mut worker);
        install(&mut worker, &output, &script);
        output.take();
        let generation = worker.generation;
        finish(&mut worker, &output, &script, &wakes, Err(outcome));
        let message = format!("{} {said}", cause());
        assert_eq!(
            notices(&output.take()),
            [notice(message.clone(), true, false)]
        );
        assert_eq!(worker.status, message);
        assert_eq!(worker.generation, generation, "nothing opened again");
        // Install… asks again.
        install(&mut worker, &output, &script);
    }
}

#[test]
fn authorization_that_became_unavailable_leaves_the_manual_step() {
    // Without an agent in the session, the notice says why Install… went.
    for (outcome, expected) in [
        (SandboxProfileError::Unavailable, REFUSED.to_owned()),
        (
            SandboxProfileError::NoAgent,
            format!("{REFUSED} {NO_AGENT}"),
        ),
    ] {
        let script = scripted();
        let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
        let wakes = wakes(&mut worker);
        install(&mut worker, &output, &script);
        output.take();
        finish(&mut worker, &output, &script, &wakes, Err(outcome));
        assert_eq!(
            notices(&output.take()),
            [notice(expected.clone(), false, false)]
        );
        assert_eq!(worker.status, expected);
        worker.input(ViewerInput::InstallSandboxPermission, &output);
        assert!(
            script
                .starts
                .recv_timeout(Duration::from_millis(50))
                .is_err()
        );
    }
}

#[test]
fn a_refusal_after_a_successful_installation_is_not_offered_again() {
    let script = scripted();
    let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
    let wakes = wakes(&mut worker);
    install(&mut worker, &output, &script);
    finish(&mut worker, &output, &script, &wakes, Ok(()));
    output.take();
    // The browser of the document opened again is refused once more.
    show(&mut worker, &output, Some(refusal()));
    let message = format!("{} {STILL_REFUSED}", cause());
    assert_eq!(notices(&output.take()), [notice(message, false, false)]);
    assert_eq!(script.offers.load(Ordering::SeqCst), 1);
}

#[test]
fn another_document_removes_the_notice_and_a_later_outcome_shows_nothing() {
    for outcome in [Err(SandboxProfileError::Declined), Ok(())] {
        let script = scripted();
        let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
        let wakes = wakes(&mut worker);
        install(&mut worker, &output, &script);
        output.take();
        worker.open(&output);
        assert_eq!(notices(&output.take()), [None]);
        let generation = worker.generation;
        finish(&mut worker, &output, &script, &wakes, outcome);
        assert!(notices(&output.take()).is_empty());
        assert_eq!(
            worker.generation, generation,
            "only a refused document opens again"
        );
    }
}

#[test]
fn closing_the_window_or_its_worker_cancels_a_pending_authorization() {
    let script = scripted();
    let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
    let cancel = install(&mut worker, &output, &script);
    worker.window_closed.store(true, Ordering::Release);
    assert!(cancel.is_cancelled());
    let script = scripted();
    let (mut worker, output) = refused(Some(refusal()), script.offer.clone());
    let cancel = install(&mut worker, &output, &script);
    drop(worker);
    assert!(cancel.is_cancelled());
}
