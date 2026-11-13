use super::*;
use crate::{Input, Output, Service, Session};
use midnight_core::{DirectoryIndex, Entry, EntryId, FsError, ViewerAppearance, ViewerGeometry};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

fn launch(sources: Vec<FileCompareSource>) -> FileCompareLaunch {
    FileCompareLaunch {
        sources,
        disabled_plugins: Arc::from([]),
    }
}

fn open(service: &Service, launch: FileCompareLaunch) -> Session {
    service
        .open(
            launch,
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [30, 30, 30, 255],
                selection: [50, 80, 120, 255],
                accent: [90, 150, 220, 255],
            },
            ViewerGeometry {
                width: 640,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap()
}

fn next_result(session: &Session) -> Output {
    eprintln!(
        "Comparator interactive waiter test={:?} mailbox={:p}",
        std::thread::current().name(),
        session.mailbox.as_ref()
    );
    let start = Instant::now();
    let mut last_status = None;
    loop {
        match session.mailbox.take() {
            Some(Output::Status(status)) => {
                eprintln!("Comparator status after {:?}: {status}", start.elapsed());
                last_status = Some(status);
            }
            None => {}
            Some(result) => {
                eprintln!("Comparator interactive result after {:?}", start.elapsed());
                return result;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "No interactive result from comparator after {:?}; last status: {last_status:?}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn choice(session: &Session) -> FileChoice {
    match next_result(session) {
        Output::ChooseFiles(choice) => choice,
        _ => panic!("Expected an editable file choice"),
    }
}

#[test]
fn incomplete_pair_preserves_the_known_side_and_never_substitutes_an_opposite_cursor() {
    for active in 0..2 {
        let known = PathBuf::from("known/Žluťoučký 世界.txt");
        let mut sources = vec![FileCompareSource::Unspecified; 2];
        sources[active] = FileCompareSource::Focused(known.clone());
        let partial = resolve(&launch(sources), &AtomicBool::new(false)).unwrap_err();
        assert_eq!(partial.paths[active], known);
        assert!(partial.paths[1 - active].as_os_str().is_empty());
    }
}

#[test]
fn missing_same_name_preserves_selected_source_but_does_not_pick_a_different_file() {
    let directory = PathBuf::from("opposite");
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(0),
        directory.join("unrelated.txt"),
        EntryKind::File,
    )]
    .into();
    let partial = resolve(
        &launch(vec![
            FileCompareSource::SameName {
                directory,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            },
            FileCompareSource::Focused("active/chosen.txt".into()),
        ]),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(partial.paths[0].as_os_str().is_empty());
    assert_eq!(partial.paths[1], PathBuf::from("active/chosen.txt"));
    assert!(partial.message.contains("same-name"));
}

#[test]
fn manual_choice_retries_invalid_paths_and_compares_real_unicode_files_without_modifying_them() {
    let directory = tempfile::tempdir().unwrap();
    let paths = [
        directory.path().join("Žluťoučký.txt"),
        directory.path().join("世界.txt"),
    ];
    let bytes = ["old 🐈\n", "new 🐈\n"];
    for side in 0..2 {
        std::fs::write(&paths[side], bytes[side]).unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::new(runtime.handle().clone());
    let session = open(
        &service,
        launch(vec![
            FileCompareSource::Focused(paths[0].clone()),
            FileCompareSource::Unspecified,
        ]),
    );
    assert_eq!(choice(&session).paths, [paths[0].clone(), PathBuf::new()]);
    for invalid in [
        PathBuf::new(),
        PathBuf::from("relative.txt"),
        directory.path().to_owned(),
        directory.path().join("missing"),
    ] {
        let requested = [paths[0].clone(), invalid];
        assert!(session.send(Input::Compare(requested.clone())));
        let rejected = choice(&session);
        assert_eq!(rejected.paths, requested);
        assert!(rejected.message.contains("File B"), "{}", rejected.message);
    }
    assert!(session.send(Input::Resize(
        ViewerGeometry {
            width: 360,
            height: 160,
            scale: 1.0
        },
        42
    )));
    assert!(session.send(Input::Compare(paths.clone())));
    let (frame, names) = match next_result(&session) {
        Output::Frame { frame, names } => (frame, names),
        Output::ChooseFiles(choice) => panic!("Expected text comparison: {choice:?}"),
        Output::Status(status) => panic!("Expected text comparison: {status}"),
    };
    assert!(frame.accessible_text.contains("old 🐈") && frame.accessible_text.contains("new 🐈"));
    assert_eq!(
        (frame.width, frame.height, frame.geometry_revision),
        (360, 160, 42)
    );
    assert_eq!(
        names,
        paths.each_ref().map(|path| path.display().to_string())
    );
    session.close();
    for side in 0..2 {
        assert_eq!(std::fs::read(&paths[side]).unwrap(), bytes[side].as_bytes());
    }
}

struct FakePicker {
    calls: mpsc::Sender<(PathPickerRequest, std::thread::ThreadId)>,
    result: Option<PathBuf>,
    wait_for_cancel: bool,
    done: mpsc::Sender<()>,
}
impl PathPickerService for FakePicker {
    fn pick_path(
        &self,
        request: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        self.calls
            .send((request.clone(), std::thread::current().id()))
            .unwrap();
        if self.wait_for_cancel {
            let start = Instant::now();
            while !cancel.is_cancelled() {
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "Closing comparator did not cancel picker"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.done.send(()).unwrap();
        Ok(self.result.clone())
    }
}

#[test]
fn browse_is_off_thread_preserves_other_side_and_cancelled_browse_keeps_fields() {
    let directory = tempfile::tempdir().unwrap();
    let paths = [
        directory.path().join("α.txt"),
        directory.path().join("β.txt"),
    ];
    for side in 0..2 {
        for result in [None, Some(directory.path().join("picked 世界.txt"))] {
            let (calls, called) = mpsc::channel();
            let (done, finished) = mpsc::channel();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let service =
                Service::new(runtime.handle().clone()).with_path_picker(Arc::new(FakePicker {
                    calls,
                    done,
                    result: result.clone(),
                    wait_for_cancel: false,
                }));
            let session = open(&service, launch(Vec::new()));
            choice(&session);
            let owner = PathPickerOwner::X11(123);
            assert!(session.send(Input::Browse {
                side,
                paths: paths.clone(),
                owner
            }));
            let chosen = choice(&session);
            let (request, thread) = called.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_ne!(thread, std::thread::current().id());
            assert_eq!(request.owner, owner);
            assert_eq!(request.kind, PathPickerKind::File);
            assert_eq!(request.initial_directory.as_deref(), Some(directory.path()));
            assert_eq!(chosen.paths[1 - side], paths[1 - side]);
            assert_eq!(
                chosen.paths[side],
                result.unwrap_or_else(|| paths[side].clone())
            );
            assert!(chosen.message.is_empty());
            finished.recv_timeout(Duration::from_secs(2)).unwrap();
            session.close();
        }
    }
}

#[test]
fn closing_comparator_cancels_a_running_native_picker() {
    let (calls, called) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::new(runtime.handle().clone()).with_path_picker(Arc::new(FakePicker {
        calls,
        done,
        result: None,
        wait_for_cancel: true,
    }));
    let session = open(&service, launch(Vec::new()));
    choice(&session);
    assert!(session.send(Input::Browse {
        side: 0,
        paths: Default::default(),
        owner: PathPickerOwner::None
    }));
    called.recv_timeout(Duration::from_secs(2)).unwrap();
    session.close();
    finished.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn unavailable_picker_and_malformed_picker_result_remain_recoverable() {
    let paths = [PathBuf::from("keep A"), PathBuf::from("keep B")];
    let no_service = Picker {
        service: None,
        cancel: Cancellation::default(),
    };
    let failure = no_service.browse(0, paths.clone(), PathPickerOwner::None);
    assert_eq!(failure.paths, paths);
    assert!(!failure.message.is_empty());
    let (calls, _called) = mpsc::channel();
    let (done, _finished) = mpsc::channel();
    let picker = Picker {
        service: Some(Arc::new(FakePicker {
            calls,
            done,
            result: Some("relative".into()),
            wait_for_cancel: false,
        })),
        cancel: Cancellation::default(),
    };
    for side in [0, 1, usize::MAX] {
        let failure = picker.browse(side, paths.clone(), PathPickerOwner::None);
        assert_eq!(failure.paths, paths);
        assert!(!failure.message.is_empty());
    }
}
