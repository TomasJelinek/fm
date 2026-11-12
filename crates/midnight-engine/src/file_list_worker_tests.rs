use super::*;
use std::collections::HashSet;

fn request(names: &[&str], template: &str) -> FileListRequest {
    let entries: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(index, name)| Entry::new(EntryId(index as u64 + 1), (*name).into(), EntryKind::File))
        .collect::<Vec<_>>()
        .into();
    let selected: Arc<HashSet<_>> = Arc::new(entries.iter().map(|e| e.id).collect());
    FileListRequest {
        id: DesktopRequestId(1),
        generation: 1,
        template: template.into(),
        destination: FileListDestination::Preview,
        capture: Arc::new(FileListCapture {
            directory: String::new(),
            source: FileListSource::Archive {
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                selected,
                fallback: None,
            },
        }),
    }
}

#[test]
fn preview_is_bounded_and_max_width_measures_rows_outside_preview() {
    let names = (0..20)
        .map(|index| format!("file-{index}.txt"))
        .collect::<Vec<_>>();
    let mut names = names.iter().map(String::as_str).collect::<Vec<_>>();
    names[19] = "longest-name-outside-preview.txt";
    let result = generate(
        &request(&names, "$(FileName:max)|$(LF)"),
        &Cancellation::default(),
    )
    .unwrap();
    let Generated::Preview {
        text,
        items,
        truncated,
    } = result
    else {
        panic!()
    };
    assert_eq!(items, 20);
    assert!(truncated);
    assert_eq!(text.lines().count(), 12);
    assert_eq!(
        text.lines().next().unwrap(),
        "file-0.txt                      |"
    );
    assert!(!text.contains("longest-name"));
}

#[test]
fn preview_truncation_preserves_unicode_boundaries_and_total_count() {
    let name = "文".repeat(10_000);
    let result = generate(&request(&[&name], "$(FileName)"), &Cancellation::default()).unwrap();
    let Generated::Preview {
        text,
        items,
        truncated,
    } = result
    else {
        panic!()
    };
    assert_eq!(items, 1);
    assert!(truncated);
    assert_eq!(text.len(), 16_383);
    assert!(text.chars().all(|value| value == '文'));
}

#[test]
fn malformed_templates_empty_selection_and_cancelled_generation_fail_without_output() {
    assert!(generate(&request(&["file.txt"], ""), &Cancellation::default()).is_err());
    assert!(
        generate(
            &request(&["file.txt"], "$(NotAVariable)"),
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(generate(&request(&[], "$(FileName)"), &Cancellation::default()).is_err());
    let cancel = Cancellation::default();
    cancel.cancel();
    let mut request = request(&["file.txt"], "$(FileName)");
    request.destination = FileListDestination::Clipboard;
    assert!(generate(&request, &cancel).is_err());
}

#[test]
fn unicode_output_and_explicit_line_endings_are_exact() {
    let request = request(
        &["Žluťoučký.txt", "文档.rs"],
        "$(FileName)$(TAB)$(FileExtension)$(CRLF)",
    );
    let Generated::Preview {
        text,
        items,
        truncated,
    } = generate(&request, &Cancellation::default()).unwrap()
    else {
        panic!()
    };
    assert_eq!(text, "Žluťoučký.txt\ttxt\r\n文档.rs\trs\r\n");
    assert_eq!(items, 2);
    assert!(!truncated);
}

#[test]
fn file_list_retirement_completed_unconsumed_generated_spool_retires_off_caller() {
    struct Probe(std::sync::mpsc::Sender<std::thread::ThreadId>);
    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (dropped, observe) = std::sync::mpsc::channel();
    let task = runtime.spawn_blocking(move || {
        let mut request = request(&["文档.txt"], "$(FileName)$(LF)");
        request.destination = FileListDestination::Viewer;
        let mut output = generate(&request, &Cancellation::default()).unwrap();
        let Generated::Content { source, .. } = &mut output else {
            panic!("generated spool")
        };
        Arc::get_mut(source)
            .unwrap()
            .observe_drop(Box::new(Probe(dropped)));
        output
    });
    // One worker and a later marker prove the first result is complete without
    // polling or consuming its JoinHandle on the current-thread runtime.
    let (finished, barrier) = std::sync::mpsc::channel();
    let marker = runtime.spawn_blocking(move || {
        finished.send(()).unwrap();
    });
    barrier
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(task.is_finished());
    drop(task);
    drop(marker);
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    let on = observe
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_ne!(
        on,
        std::thread::current().id(),
        "completed generated Spool closed on caller"
    );
}

#[test]
fn generated_spool_cleanup_retains_admission_through_blocked_or_panicking_drop() {
    struct Gate {
        entered: std::sync::mpsc::Sender<std::thread::ThreadId>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        panic: bool,
    }
    impl Drop for Gate {
        fn drop(&mut self) {
            let _ = self.entered.send(std::thread::current().id());
            let _ = self
                .release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5));
            assert!(!self.panic, "injected post-Spool destructor panic");
        }
    }
    for panic in [false, true] {
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        let quota = Arc::new(tokio::sync::Semaphore::new(1));
        let token = Arc::new(Retention {
            _permit: quota.clone().try_acquire_owned().unwrap(),
        });
        let retained_job = token.clone();
        let mut request = request(&["文档.txt"], "$(FileName)");
        request.destination = FileListDestination::Viewer;
        let Generated::Content { mut source, .. } = generate_with_sizes(
            &request,
            &Cancellation::default(),
            &Captured::default(),
            OutputReservation {
                lease: domain.try_reserve().unwrap(),
                retention: token,
            },
        )
        .unwrap() else {
            panic!("actual generated spool")
        };
        let (entered, entry) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        Arc::get_mut(&mut source)
            .unwrap()
            .observe_drop(Box::new(Gate {
                entered,
                release: std::sync::Mutex::new(wait),
                panic,
            }));
        drop(source);
        let on = entry
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_ne!(on, std::thread::current().id());
        assert!(
            domain.try_reserve().is_err(),
            "cleanup still owns native payload slot"
        );
        assert_eq!(quota.available_permits(), 0);
        drop(retained_job);
        assert_eq!(
            quota.available_permits(),
            0,
            "last Job release cannot release live output quota"
        );
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if quota.available_permits() == 1
                && let Ok(lease) = domain.try_reserve()
            {
                drop(lease);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "cleanup did not release admission"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
