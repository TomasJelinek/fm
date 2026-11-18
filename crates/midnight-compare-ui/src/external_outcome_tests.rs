//! Typed external outcomes retain uncertainty, native drafts and terminal priority.
use super::{Input, OpenWindow, file_choice, identity_tests, raw_paths};
use midnight_compare_engine::{
    ExternalCompareChoiceOutcome as Outcome, FileChoice, IdentityVerdict, Output,
};
use midnight_core::{PathPickerOwner, ViewerGeometry};
use slint::{ComponentHandle, platform::WindowEvent};
use std::{path::PathBuf, rc::Rc};

fn open(runtime: &identity_tests::TestRuntime) -> Rc<OpenWindow> {
    bind_open(identity_tests::open(runtime))
}

fn bind_open(open: Rc<OpenWindow>) -> Rc<OpenWindow> {
    file_choice::bind(&open, |_| PathPickerOwner::None);
    open.window.set_picker_available(true);
    open
}

fn choice(paths: [PathBuf; 2], external_outcome: Option<Outcome>, message: &str) -> Output {
    Output::ChooseFiles(FileChoice {
        paths,
        message: message.into(),
        is_error: true,
        external_outcome,
    })
}

fn request_choice(open: &OpenWindow, side: Option<usize>) {
    if let Some(side) = side {
        open.window.invoke_browse_file(side as i32);
    } else {
        open.window.invoke_compare_chosen_files();
    }
}

struct CancelPicker(std::sync::mpsc::SyncSender<midnight_core::PathPickerRequest>);

impl midnight_core::PathPickerService for CancelPicker {
    fn pick_path(
        &self,
        request: &midnight_core::PathPickerRequest,
        _cancel: &midnight_core::Cancellation,
    ) -> Result<Option<PathBuf>, midnight_core::FsError> {
        self.0.try_send(request.clone()).unwrap();
        Ok(None)
    }
}

fn assert_unknown_warning(open: &OpenWindow) {
    assert_eq!(open.external_outcome.get(), Some(Outcome::Unknown));
    let message = open.window.get_choice_message().to_ascii_lowercase();
    assert!(message.contains("may have started"), "{message}");
    assert!(message.contains("check the other application"), "{message}");
    assert!(message.contains("before retry"), "{message}");
}

#[test]
fn typed_external_outcomes_override_misleading_prose_without_reclassifying_ordinary_choices() {
    let _native = identity_tests::platform();
    let runtime = identity_tests::runtime();
    let open = open(&runtime);
    let paths = identity_tests::paths();
    let ordinary = "Unknown outcome: the program may have started. Check before retrying.";
    open.present_output(&choice(paths.clone(), None, ordinary));
    assert_eq!(open.external_outcome.get(), None);
    assert_eq!(open.window.get_choice_message(), ordinary);
    assert_eq!(open.window.get_status(), ordinary);

    open.present_output(&choice(
        paths.clone(),
        Some(Outcome::NotStarted),
        "Started successfully; no uncertainty remains.",
    ));
    assert_eq!(open.external_outcome.get(), Some(Outcome::NotStarted));
    assert!(
        open.window
            .get_choice_message()
            .to_ascii_lowercase()
            .contains("did not start")
    );
    assert!(!open.closed.get());

    open.present_output(&choice(
        paths.clone(),
        Some(Outcome::Unknown),
        "Not started. It is safe to retry immediately.",
    ));
    assert_unknown_warning(&open);
    assert!(open.window.get_choosing_files());
    assert_eq!(open.window.get_status(), open.window.get_choice_message());
    assert_eq!(*open.choice_paths.borrow(), paths);
    assert!(!open.closed.get());
    open.close();
}

#[test]
fn unknown_outcome_survives_resize_passive_outputs_chooser_focus_and_repaint() {
    let native = identity_tests::platform();
    let runtime = identity_tests::runtime();
    let open = open(&runtime);
    let paths = identity_tests::paths();
    open.present_output(&identity_tests::frame(7, IdentityVerdict::IdenticalFiles));
    open.present_output(&choice(
        paths.clone(),
        Some(Outcome::Unknown),
        "The outcome observer disconnected.",
    ));
    let warning = open.window.get_choice_message();
    assert_unknown_warning(&open);
    assert!(open.send(Input::Resize(
        ViewerGeometry {
            width: 640,
            height: 480,
            scale: 1.0,
        },
        4,
    )));
    let replacement_paths = [
        PathBuf::from("replacement-a"),
        PathBuf::from("replacement-b"),
    ];
    for output in [
        Output::Status("Finished; safe to retry without checking.".into()),
        identity_tests::frame(0, IdentityVerdict::IdenticalFiles),
        identity_tests::frame(7, IdentityVerdict::IdenticalText),
        identity_tests::frame(8, IdentityVerdict::Different),
        choice(replacement_paths.clone(), None, "Choose another pair"),
        choice(
            replacement_paths.clone(),
            Some(Outcome::NotStarted),
            "A later status claims no process started.",
        ),
        choice(
            replacement_paths,
            Some(Outcome::Unknown),
            "A replacement chooser must not replace the retained draft.",
        ),
    ] {
        open.present_output(&output);
        assert_unknown_warning(&open);
        assert_eq!(open.window.get_choice_message(), warning);
        assert_eq!(open.window.get_status(), warning);
        assert_eq!(*open.choice_paths.borrow(), paths);
        assert_eq!(
            open.window.get_file_a(),
            paths[0].to_string_lossy().as_ref()
        );
        assert_eq!(
            open.window.get_file_b(),
            paths[1].to_string_lossy().as_ref()
        );
        assert!(open.window.get_choosing_files());
        assert!(!open.window.get_choice_busy());
        assert_eq!(open.window.get_identity_state(), -1);
        assert_eq!(open.window.get_surface().size().width, 0);
        assert_eq!(open.comparison_generation.get(), 7);
    }

    open.window.invoke_focus_files();
    open.window.set_compact(true);
    open.window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(320, 200));
    open.window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(320.0, 200.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 320 * 200];
    slint::platform::update_timers_and_animations();
    open.window.window().request_redraw();
    assert!(native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 320);
    }));
    assert_unknown_warning(&open);
    assert_eq!(open.window.get_choice_message(), warning);
    assert_eq!(open.window.get_status(), warning);
    assert_eq!(*open.choice_paths.borrow(), paths);
    assert!(open.window.get_choosing_files());
    open.close();
}

#[test]
fn only_session_admitted_compare_or_browse_clears_unknown() {
    let _native = identity_tests::platform();
    let runtime = identity_tests::runtime();
    let paths = identity_tests::paths();
    for side in [None, Some(0), Some(1)] {
        for closed in [false, true] {
            let open = open(&runtime);
            open.present_output(&choice(
                paths.clone(),
                Some(Outcome::Unknown),
                "Launch completion is uncertain.",
            ));
            if closed {
                open.session.close();
            }
            assert!(!open.closed.get());
            request_choice(&open, side);
            assert!(open.pending.borrow().is_empty());
            if closed {
                assert_unknown_warning(&open);
                assert!(!open.window.get_choice_busy());
                assert!(open.window.get_choice_error());
                assert!(!open.comparison_requested.get());
            } else {
                assert_eq!(open.external_outcome.get(), None);
                assert!(open.window.get_choice_busy());
                assert_eq!(open.comparison_requested.get(), side.is_none());
                let result = identity_tests::next_choice(&open);
                assert_eq!(result.paths, paths);
                let message = result.message.clone();
                open.present_output(&Output::ChooseFiles(result));
                assert_eq!(open.external_outcome.get(), None);
                assert_eq!(open.window.get_choice_message(), message);
                assert!(!open.window.get_choice_busy());
            }
            assert_eq!(*open.choice_paths.borrow(), paths);
            assert!(open.window.get_choosing_files());
            open.close();
        }
    }
}

#[test]
fn external_started_outranks_unknown_and_session_admitted_compare_and_stays_closed() {
    let _native = identity_tests::platform();
    let runtime = identity_tests::runtime();
    for scenario in ["ready", "unknown", "admitted", "closed"] {
        let open = open(&runtime);
        open.window.show().unwrap();
        open.present_output(&identity_tests::frame(3, IdentityVerdict::IdenticalFiles));
        match scenario {
            "unknown" => open.present_output(&choice(
                identity_tests::paths(),
                Some(Outcome::Unknown),
                "No confirmed outcome was observed.",
            )),
            "admitted" => {
                assert!(open.send(Input::Compare(identity_tests::paths())));
                assert!(open.pending.borrow().is_empty());
                assert!(open.comparison_requested.get());
            }
            "closed" => open.close(),
            _ => {}
        }
        open.present_output(&Output::ExternalStarted {
            pid: 42,
            tracking_warning: (scenario == "unknown").then(|| "Tracking is unconfirmed.".into()),
        });
        assert!(open.closed.get(), "{scenario}");
        assert!(!open.window.window().is_visible(), "{scenario}");
        assert!(!open.comparison_requested.get());
        assert_eq!(open.window.get_identity_state(), -1);
        assert!(open.window.get_status().is_empty());
        assert!(open.window.get_accessible_text().is_empty());
        assert_eq!(open.window.get_surface().size().width, 0);
        let message = open.window.get_choice_message();
        for output in [
            Output::ExternalStarted {
                pid: 43,
                tracking_warning: None,
            },
            Output::Status("A delayed success status must not reopen the window.".into()),
            identity_tests::frame(4, IdentityVerdict::IdenticalFiles),
            choice(identity_tests::paths(), None, "A delayed chooser"),
        ] {
            open.present_output(&output);
            assert!(open.closed.get());
            assert!(!open.window.window().is_visible());
            assert!(open.window.get_status().is_empty());
            assert_eq!(open.window.get_identity_state(), -1);
            assert_eq!(open.window.get_surface().size().width, 0);
            assert_eq!(open.window.get_choice_message(), message);
        }
        assert!(!open.send(Input::Difference { backwards: true }));
        open.close();
        assert!(open.window.get_status().is_empty());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn raw_native_drafts_survive_typed_outcomes_and_chooser_compare_or_browse() {
    let _native = identity_tests::platform();
    let runtime = identity_tests::runtime();
    let [left, right] = raw_paths::paths();
    let paths = [left.join("before.txt"), right.join("after.txt")];
    for path in &paths {
        assert!(path.is_absolute());
        assert!(path.as_os_str().to_str().is_none());
    }
    for outcome in [Outcome::NotStarted, Outcome::Unknown] {
        for side in [None, Some(0), Some(1)] {
            let (service, selected) = identity_tests::text_service(&runtime);
            let (calls, called) = std::sync::mpsc::sync_channel(1);
            let service = service.with_path_picker(std::sync::Arc::new(CancelPicker(calls)));
            let open = bind_open(identity_tests::open_service(&service));
            open.present_output(&choice(paths.clone(), Some(outcome), "Native draft paths"));
            assert_eq!(open.external_outcome.get(), Some(outcome));
            assert_eq!(*open.choice_paths.borrow(), paths);
            assert_eq!(
                open.window.get_file_a(),
                paths[0].to_string_lossy().as_ref()
            );
            assert_eq!(
                open.window.get_file_b(),
                paths[1].to_string_lossy().as_ref()
            );
            request_choice(&open, side);
            assert_eq!(open.external_outcome.get(), None);
            assert!(open.window.get_choice_busy());
            assert!(open.pending.borrow().is_empty());
            let result = identity_tests::next_choice(&open);
            assert_eq!(result.paths, paths);
            if let Some(side) = side {
                let request = called
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                assert!(
                    request
                        .title
                        .contains(if side == 0 { "file A" } else { "file B" })
                );
                assert_eq!(request.initial_directory.as_deref(), paths[side].parent());
                assert!(selected.lock().unwrap().is_empty());
            } else {
                assert_eq!(
                    selected.lock().unwrap().as_slice(),
                    std::slice::from_ref(&paths)
                );
            }
            open.present_output(&Output::ChooseFiles(result));
            assert_eq!(*open.choice_paths.borrow(), paths);
            assert!(!open.window.get_choice_busy());
            open.close();
        }
    }
}
