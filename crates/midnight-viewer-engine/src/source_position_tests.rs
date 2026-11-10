//! Exact coordinates through the separately packaged native source plugin.
use super::*;

#[test]
#[ignore = "requires MIDNIGHT_VIEWER_TEST_INSTALL with the source Locate plugin"]
fn source_search_reports_global_scalar_position_and_preserves_copy_after_preferences() {
    let installation =
        std::path::PathBuf::from(std::env::var_os("MIDNIGHT_VIEWER_TEST_INSTALL").unwrap());
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let service = Service::new(executor.handle().clone());
    *service.runtime.lock().unwrap() = Some(Arc::new(Runtime::discover(&installation).unwrap()));
    *service.images.lock().unwrap() = Some(Arc::from([]));
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("coordinates.rs");
    let source: String = (1..=20_000)
        .map(|line| format!("// row {line:05} α😀\r\n"))
        .collect();
    std::fs::write(&path, source).unwrap();
    images::settle_generated_fixture(&path);
    let session = service
        .open(
            launch(files.path(), &["coordinates.rs"], false),
            appearance(),
            ViewerGeometry {
                width: 500,
                height: 180,
                scale: 1.0,
            },
        )
        .unwrap();
    let initial = frame(&session, |frame| frame.mode == 2);
    assert!(initial.status.starts_with("Ln 1, Col 1"));
    assert!(session.send(ViewerInput::Search {
        query: "row 02001".into(),
        backwards: false,
        case_sensitive: true
    }));
    let found = frame(&session, |frame| {
        frame.status.starts_with("Ln 2001, Col 13")
    });
    assert!(found.accessible_text.starts_with("row 02001"));
    session
        .change_preferences(&ViewerPreferenceChange::LineNumbers(false))
        .unwrap();
    let plain = frame(&session, |frame| {
        frame.revision > found.revision && frame.status.starts_with("Ln 2001, Col 13")
    });
    assert_eq!(plain.accessible_text, found.accessible_text);
    assert!(plain.pixels != found.pixels);
    assert!(session.send(ViewerInput::Copy {
        frame: plain.revision
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut copied = None;
    while Instant::now() < deadline && copied.is_none() {
        for output in session.mailbox.take() {
            if let ViewerOutput::CopyText(text) = output {
                copied = Some(text);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(copied.as_deref(), Some("row 02001"));
    session.close();
}
