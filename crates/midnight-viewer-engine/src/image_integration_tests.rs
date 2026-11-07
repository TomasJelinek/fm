use super::*;

fn settle_generated_fixture(path: &Path) {
    // Fresh BMPs in native acceptance runs can acquire a metadata-only revision shortly
    // after creation. Wait before capturing, never relax the production revision checks.
    let started = Instant::now();
    let mut quiet = Instant::now();
    let mut previous = midnight_plugin_host::CapturedFile::capture(path)
        .unwrap()
        .stable_version();
    while quiet.elapsed() < Duration::from_millis(500) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "generated image fixture did not settle"
        );
        std::thread::sleep(Duration::from_millis(20));
        let current = midnight_plugin_host::CapturedFile::capture(path)
            .unwrap()
            .stable_version();
        if current != previous {
            previous = current;
            quiet = Instant::now();
        }
    }
}

fn bitmap() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend_from_slice(&60054u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&200i32.to_le_bytes());
    bytes.extend_from_slice(&100i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for y in 0..100u8 {
        for x in 0..200u8 {
            bytes.extend_from_slice(&[x, y, 255]);
        }
    }
    bytes
}

#[test]
#[ignore = "requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION with trusted native and image packages"]
fn image_worker_routes_fits_zooms_pans_cycles_and_preserves_basic_override() {
    let application = std::path::PathBuf::from(
        std::env::var_os("MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION").unwrap(),
    );
    let runtime = Arc::new(Runtime::discover(application.parent().unwrap()).unwrap());
    let providers = midnight_image_viewer_host::discover(&application).unwrap();
    assert!(!providers.is_empty());
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let service = Service {
        handle: executor.handle().clone(),
        runtime: Arc::new(Mutex::new(Some(runtime))),
        images: Arc::new(Mutex::new(Some(providers.into()))),
        windows: Arc::new(AtomicUsize::new(0)),
    };
    let files = tempfile::tempdir().unwrap();
    std::fs::write(files.path().join("first.BMP"), bitmap()).unwrap();
    std::fs::write(files.path().join("second.txt"), "ordinary text").unwrap();
    settle_generated_fixture(&files.path().join("first.BMP"));
    let geometry = ViewerGeometry {
        width: 100,
        height: 60,
        scale: 1.0,
    };
    let session = service
        .open(
            launch(files.path(), &["first.BMP", "second.txt"], false),
            appearance(),
            geometry,
        )
        .unwrap();
    let fitted = frame(&session, |frame| frame.image.is_some());
    let image = fitted.image.as_ref().unwrap();
    assert_eq!(
        (image.width, image.height, image.zoom, image.fitted),
        (200, 100, 0.5, true)
    );
    assert_eq!(fitted.mode, midnight_core::VIEWER_MODE_IMAGE);
    assert!(!fitted.wrap_available);
    assert_eq!(&fitted.pixels[..4], &[0; 4], "letterbox is transparent");
    session.send(ViewerInput::Image {
        frame: fitted.revision,
        input: midnight_core::ViewerImageInput::ActualPixels,
    });
    let actual = frame(&session, |frame| {
        frame.image.as_ref().is_some_and(|image| image.zoom == 1.0)
    });
    assert_eq!(actual.horizontal_extent, 0.5);
    session.send(ViewerInput::Pan(1.0));
    let panned = frame(&session, |frame| frame.revision > actual.revision);
    assert_eq!(
        panned.image.as_ref().unwrap().zoom,
        1.0,
        "panning preserves physical zoom"
    );
    assert_eq!(panned.horizontal_position, 1.0);
    session.send(ViewerInput::Image {
        frame: panned.revision,
        input: midnight_core::ViewerImageInput::Zoom {
            factor: 1.25,
            anchor: Some([50.0, 30.0]),
        },
    });
    session.send(ViewerInput::Image {
        frame: panned.revision,
        input: midnight_core::ViewerImageInput::Zoom {
            factor: 1.25,
            anchor: Some([50.0, 30.0]),
        },
    });
    let zoomed = frame(&session, |frame| {
        frame
            .image
            .as_ref()
            .is_some_and(|image| image.zoom == 1.5625)
    });
    assert_eq!(
        zoomed.image.as_ref().unwrap().zoom,
        1.5625,
        "a burst from one displayed frame keeps every zoom step"
    );
    session.send(ViewerInput::Image {
        frame: zoomed.revision,
        input: midnight_core::ViewerImageInput::Fit,
    });
    frame(&session, |frame| {
        frame.image.as_ref().is_some_and(|image| image.fitted)
    });
    session.send(ViewerInput::NextFile);
    let text = frame(&session, |frame| frame.caption.contains("second.txt"));
    assert!(text.image.is_none());
    assert_eq!(&*text.accessible_text, "ordinary text");
    session.send(ViewerInput::NextFile);
    let cycled = frame(&session, |frame| frame.image.is_some());
    session.send(ViewerInput::Image {
        frame: zoomed.revision,
        input: midnight_core::ViewerImageInput::ActualPixels,
    });
    session.send(ViewerInput::Pan(0.0));
    let unchanged = frame(&session, |frame| frame.revision > cycled.revision);
    assert_eq!(
        unchanged.image.as_ref().unwrap().zoom,
        0.5,
        "previous-image input is rejected"
    );
    session.close();
    let started = Instant::now();
    while service.windows.load(Ordering::Acquire) != 0 && started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(service.windows.load(Ordering::Acquire), 0);
    let basic = service
        .open(
            launch(files.path(), &["first.BMP"], true),
            appearance(),
            geometry,
        )
        .unwrap();
    let forced = frame(&basic, |_| true);
    assert!(forced.image.is_none());
    assert_eq!(forced.mode, midnight_plugin_host::api::viewer::MODE_TEXT);
    basic.close();
    let mut disabled = launch(files.path(), &["first.BMP"], false);
    disabled.disabled_plugins = Arc::from(["midnight.viewer.image".into()]);
    let fallback = service.open(disabled, appearance(), geometry).unwrap();
    let hex = frame(&fallback, |_| true);
    assert!(hex.image.is_none());
    assert_eq!(hex.mode, midnight_plugin_host::api::viewer::MODE_BINARY);
    fallback.close();
    let mut duplicate = service.images.lock().unwrap().as_ref().unwrap().to_vec();
    duplicate[0].id = "midnight.viewer.basic".into();
    *service.images.lock().unwrap() = Some(duplicate.into());
    let ambiguous = service
        .open(
            launch(files.path(), &["first.BMP"], false),
            appearance(),
            geometry,
        )
        .unwrap();
    status(&ambiguous, "no compatible viewer");
    ambiguous.close();
}
