//! U-D13 of the UI polish gate: at their own minimum size, in Blue Desktop and Dark
//! Touch, a job window and a decision window cut no text off and keep every shown
//! caption whole: a long waiting label stays inside the speed chart, a finished job's
//! footer leaves out Copy details rather than cutting its caption, and a failure's
//! Retry, Skip and Skip all fit side by side. The rows come from the presenter, so the
//! labels are the ones the windows really show.
use super::outcome_notices::deletion;
use super::polish;
use super::*;
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use midnight_ui::{DecisionWindow, JobProgressWindow, OperationRow};

/// An appearance: its name, theme, density and font size.
type Appearance = (&'static str, i32, i32, i32);

/// Blue Desktop and Dark Touch with their default fonts, as the gate captures job
/// windows.
const APPEARANCES: [Appearance; 2] = [("Blue Desktop", 0, 0, 12), ("Dark Touch", 1, 1, 15)];

fn row(h: &Harness, identity: &str) -> OperationRow {
    h.main
        .get_operations()
        .iter()
        .find(|row| row.identity == identity)
        .unwrap_or_else(|| panic!("{identity} is listed"))
}

fn denied(path: &str) -> FsError {
    FsError::from_io(
        "read",
        Path::new(path),
        io::Error::from(io::ErrorKind::PermissionDenied),
    )
}

/// A copy of `id` in `state`, as the backend reports it.
fn copy(id: u64, state: CopyOperationState, edit: impl FnOnce(&mut CopyProgress)) -> CopyProgress {
    let mut progress = CopyProgress::queued(
        OperationId(id),
        "destination".into(),
        1,
        TransferIntent::Copy,
    );
    progress.state = state;
    edit(&mut progress);
    progress
}

/// The rows of the job states the gate captures at the minimum size, and two more.
fn job_states(h: &Harness) -> Vec<(&'static str, OperationRow)> {
    let mut states = Vec::new();
    let long = "source/a folder with a rather long name/資料/another-file-with-a-long-name.bin";
    for (id, what, progress) in [
        (
            1,
            "waiting for earlier transfers",
            copy(1, CopyOperationState::Queued, |p| {
                p.wait_reason = Some(TransferWaitReason::EarlierTransfers);
            }),
        ),
        (
            2,
            "waiting for a job in the same folders",
            copy(2, CopyOperationState::Running, |p| {
                p.wait_reason = Some(TransferWaitReason::SameFolders);
            }),
        ),
        (
            3,
            "running",
            copy(3, CopyOperationState::Running, |p| {
                p.phase = CopyOperationPhase::Copying;
                p.current_path = Some(long.into());
            }),
        ),
        (
            4,
            "paused",
            copy(4, CopyOperationState::Running, |p| {
                p.paused = true;
                p.pause_requested = true;
                p.current_path = Some(long.into());
            }),
        ),
        (
            5,
            "completed with 3 errors",
            copy(5, CopyOperationState::CompletedWithErrors, |p| {
                p.files_completed = 4;
                p.failed = 3;
                for index in 0..3 {
                    p.errors
                        .push(denied(&format!("source/bad/secret-{index}.txt")));
                }
                p.message = p.errors[0].to_string();
            }),
        ),
        (
            6,
            "completed with 40 errors",
            copy(6, CopyOperationState::CompletedWithErrors, |p| {
                p.files_completed = 1;
                p.failed = 40;
                for index in 0..16 {
                    p.errors.push(denied(&format!(
                        "source/many-bad/an-unreadable-file-with-a-long-name-number-{index:02}.txt"
                    )));
                }
                p.message = p.errors[0].to_string();
            }),
        ),
        (
            7,
            "cancelled",
            copy(7, CopyOperationState::Cancelled, |p| {
                p.files_completed = 3;
                p.message = "Copy cancelled; files already copied are kept.".into();
            }),
        ),
    ] {
        h.start(id);
        h.apply(Event::Copy(CopyEvent::Progress(Arc::new(progress))));
        states.push((what, row(h, &format!("copy-job:{id}"))));
    }
    let mut failed = deletion(90, OperationState::CompletedWithErrors, "");
    failed.failed = 1;
    failed.errors.push(denied("source/資料.txt"));
    failed.message = failed.errors[0].to_string();
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(failed))));
    states.push(("a delete with an error", row(h, "90")));
    states
}

/// The native adapter of the window created last.
fn newest(h: &Harness) -> Rc<MinimalSoftwareWindow> {
    h.windows.borrow().last().cloned().expect("a window")
}

/// Lay the window out at its own minimum size in each appearance and return what the
/// gate's layout checks find there. `apply` takes a theme, a density and the desktop
/// and touch font sizes.
fn findings_at_minimum(
    native: &MinimalSoftwareWindow,
    apply: &dyn Fn(i32, i32, i32, i32),
    appearances: &[Appearance],
    what: &str,
) -> Vec<String> {
    let mut findings = Vec::new();
    for &(appearance, scheme, density, font) in appearances {
        apply(scheme, density, font, font);
        modeless::layout(native, 900, 700);
        let (width, height) = polish::minimum_size(native.window());
        modeless::draw(
            native,
            width,
            height,
            &format!("minimum-{what}-{scheme}-{font}"),
        );
        slint::platform::update_timers_and_animations();
        modeless::layout(native, width, height);
        findings.extend(
            polish::layout_findings(native.window())
                .into_iter()
                .map(|finding| {
                    format!("{what}, {appearance} {font}px at {width}x{height}: {finding}")
                }),
        );
    }
    findings
}

fn progress_window(job: &OperationRow, asking: bool) -> JobProgressWindow {
    let window = JobProgressWindow::new().unwrap();
    window.set_job(job.clone());
    window.set_cancel_confirmation(asking);
    window.show().unwrap();
    window
}

#[test]
fn job_windows_cut_nothing_and_keep_captions_whole_at_their_minimum_size() {
    let h = Harness::new();
    let states = job_states(&h);
    let mut findings = Vec::new();
    for (what, job) in states {
        for asking in [false, true] {
            // Only an active job asks whether to cancel it.
            if asking && !job.active {
                continue;
            }
            let window = progress_window(&job, asking);
            let native = newest(&h);
            let apply = |scheme, density, desktop, touch| {
                window.invoke_apply_preferences(scheme, density, "".into(), desktop, touch);
            };
            let what = if asking {
                format!("{what}, asking to cancel")
            } else {
                what.to_owned()
            };
            findings.extend(findings_at_minimum(&native, &apply, &APPEARANCES, &what));
            window.hide().unwrap();
        }
    }
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn decision_windows_cut_nothing_and_keep_captions_whole_at_their_minimum_size() {
    let h = Harness::new();
    h.start(1);
    h.failure(1, 1);
    assert!(
        h.main.get_copy_is_failure(),
        "the failure decision is shown"
    );
    let mut findings = Vec::new();
    for (what, failure, delete) in [
        ("copy failure", true, false),
        ("delete failure", true, true),
        ("name conflict", false, false),
    ] {
        // As decision_windows.rs fills it.
        let window = DecisionWindow::new().unwrap();
        window.set_token("decision".into());
        window.set_failure_mode(failure);
        if failure {
            window.set_title_text("Operation error".into());
            window.set_body(h.main.get_copy_body());
            window.set_can_retry(h.main.get_copy_can_retry());
            window.set_can_skip(h.main.get_copy_can_skip());
            window.set_allow_alternate_name(false);
            if delete {
                window.set_failure_title("Delete needs attention".into());
                window.set_skip_all_text("Skip all errors".into());
            }
        } else {
            window.set_title_text("Name conflict".into());
            window.set_body("An item named “report 文档.txt” already exists here.".into());
            window.set_initial_name("report 文档 (2).txt".into());
            window.set_allow_alternate_name(true);
            window.set_can_replace(true);
            window.set_source_details("Size: 12 KB\nModified: 2026-11-20 14:03".into());
            window.set_target_details("Size: 9 KB\nModified: 2026-11-19 08:41".into());
        }
        window.show().unwrap();
        let native = newest(&h);
        let apply = |scheme, density, desktop, touch| {
            window.invoke_apply_preferences(scheme, density, "".into(), desktop, touch);
        };
        findings.extend(findings_at_minimum(&native, &apply, &APPEARANCES, what));
        window.hide().unwrap();
    }
    assert!(findings.is_empty(), "{findings:#?}");
}

/// The gate found the waiting label cut by the speed chart with Linux's wider default
/// font; larger fonts show the same here. The label wraps inside the chart instead.
#[test]
fn a_long_waiting_label_stays_inside_the_speed_chart_with_wider_fonts() {
    let h = Harness::new();
    h.start(1);
    h.apply(Event::Copy(CopyEvent::Progress(Arc::new(copy(
        1,
        CopyOperationState::Running,
        |p| p.wait_reason = Some(TransferWaitReason::SameFolders),
    )))));
    let job = row(&h, "copy-job:1");
    let label = job.activity_label.to_string();
    assert_eq!(
        label, "Waiting for another job in the same folders to finish",
        "the longest waiting label"
    );
    let window = progress_window(&job, false);
    let native = newest(&h);
    let apply = |scheme, density, desktop, touch| {
        window.invoke_apply_preferences(scheme, density, "".into(), desktop, touch);
    };
    let mut cut = Vec::new();
    for appearance in [
        ("Blue Desktop", 0, 0, 12),
        ("Blue Desktop", 0, 0, 14),
        ("Blue Desktop", 0, 0, 16),
        ("Dark Touch", 1, 1, 15),
        ("Dark Touch", 1, 1, 18),
    ] {
        let findings = findings_at_minimum(&native, &apply, &[appearance], "waiting");
        assert!(
            modeless::accessible_items(&native, AccessibleRole::Text, &label)
                .iter()
                .any(ItemRc::is_visible),
            "{appearance:?}: the chart shows the waiting label"
        );
        cut.extend(
            findings
                .into_iter()
                .filter(|finding| finding.contains(&label)),
        );
    }
    window.hide().unwrap();
    assert!(cut.is_empty(), "{cut:#?}");
}
