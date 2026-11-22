//! Host navigation acceptance using the separately built, sandboxed PDF plugin.
use super::*;
use midnight_core::{DocumentLayout, ViewerImageInput};

fn two_page_pdf() -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
    ];
    for (index, (width, height, color)) in [(400, 600, "1 0 0"), (1000, 400, "0 0 1")]
        .into_iter()
        .enumerate()
    {
        let content = format!("{color} rg 0 0 {width} {height} re f");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << >> /Contents {} 0 R >>",
            4 + index * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.into_iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

/// The pixel at raster position `x`, `y` of the view that `frame` shows. A document is
/// published as retained tiles at the exact zoom, never as a whole-view raster: the tile
/// that covers the point supplies it (premultiplied RGBA, which equals RGBA for the opaque
/// and the empty pixels checked here).
fn view_pixel(frame: &ViewerFrame, x: u32, y: u32) -> [u8; 4] {
    assert!(
        frame.pixels.is_empty(),
        "a document frame carries tiles, not a whole-view raster"
    );
    let tiles = frame
        .tiles
        .as_ref()
        .expect("a document frame carries its retained tiles");
    let point = [f64::from(x), f64::from(y)];
    let tile = tiles
        .iter()
        .find(|tile| {
            let size = [tile.width, tile.height];
            (0..2).all(|axis| {
                let low = f64::from(tile.position[axis]);
                (low..low + f64::from(size[axis])).contains(&point[axis])
            })
        })
        .unwrap_or_else(|| panic!("no published tile covers view pixel {x}, {y}"));
    let column = (point[0] - f64::from(tile.position[0])) as usize;
    let row = (point[1] - f64::from(tile.position[1])) as usize;
    let index = (row * tile.width as usize + column) * 4;
    tile.pixels.rgba()[index..index + 4].try_into().unwrap()
}

fn change_page(session: &WindowSession, previous: &ViewerFrame, delta: i32) -> ViewerFrame {
    let page = previous.image.as_ref().unwrap().page as i32 + delta;
    assert!(session.send(ViewerInput::DocumentPage {
        frame: previous.revision,
        delta,
    }));
    frame(session, |frame| {
        frame.revision > previous.revision
            && frame
                .image
                .as_ref()
                .is_some_and(|image| image.page as i32 == page)
    })
}

fn load_pdf_catalogs(service: &Service, application: &std::path::Path) {
    let runtime = Arc::new(Runtime::discover(application.parent().unwrap()).unwrap());
    let images = midnight_image_viewer_host::discover(application).unwrap();
    assert!(
        images
            .iter()
            .any(|provider| provider.id == "midnight.viewer.pdf")
    );
    // Only the PDF image viewer is under test. Web discovery also validates the
    // application executable itself, which a private test installation lacks.
    cache_explicit_catalogs(service, (runtime, images.into(), Arc::from([])));
}

fn cache_explicit_catalogs(service: &Service, expected: crate::Catalogs) {
    *service.runtime.lock().unwrap() = Some(expected.0.clone());
    *service.images.lock().unwrap() = Some(expected.1.clone());
    *service.web.lock().unwrap() = Some(expected.2.clone());
    // All three caches must be populated before opening: a missing cache would
    // resolve the harness's installed executable instead of this explicit app.
    let cached = service.catalogs().unwrap();
    assert!(Arc::ptr_eq(&cached.0, &expected.0));
    assert!(Arc::ptr_eq(&cached.1, &expected.1));
    assert!(Arc::ptr_eq(&cached.2, &expected.2));
}

#[test]
fn explicit_pdf_fixture_catalog_cache_bypasses_installed_executable_discovery() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (_resident_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-test", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service::new(executor.handle().clone(), resident);
    let installation = tempfile::tempdir().unwrap();
    let runtime = Arc::new(Runtime::discover(installation.path()).unwrap());
    cache_explicit_catalogs(&service, (runtime.clone(), Arc::from([]), Arc::from([])));
    let cached = service.catalogs().unwrap();
    assert!(Arc::ptr_eq(&cached.0, &runtime));
    assert!(cached.1.is_empty());
    assert!(cached.2.is_empty());
}

#[test]
#[ignore = "requires MIDNIGHT_PDF_VIEWER_TEST_APPLICATION with trusted PDF package"]
fn pdf_page_navigation_retains_manual_zoom_and_fit_mode() {
    let application =
        std::path::PathBuf::from(std::env::var_os("MIDNIGHT_PDF_VIEWER_TEST_APPLICATION").unwrap());
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (_resident_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-test", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service::new(executor.handle().clone(), resident);
    load_pdf_catalogs(&service, &application);
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("mixed-pages.pdf");
    std::fs::write(&path, two_page_pdf()).unwrap();
    images::settle_generated_fixture(&path);
    let mut request = launch(files.path(), &["mixed-pages.pdf"], false);
    request.preferences = Arc::new(midnight_core::ViewerPreferences {
        document_zoom: midnight_core::ViewerZoomPreference::ScaleBasisPoints(23_456),
        ..Default::default()
    });
    let session = service
        .open(
            request,
            appearance(),
            ViewerGeometry {
                width: 160,
                height: 120,
                scale: 1.0,
            },
        )
        .unwrap();
    let mut displayed = frame(&session, |frame| frame.image.is_some());
    let initial = displayed.image.as_ref().unwrap();
    assert!(
        initial.fitted,
        "a new PDF must open in Fit despite saved zoom"
    );
    assert_eq!(initial.zoom, 0.2);
    assert_eq!(initial.document_layout, Some(DocumentLayout::Paged));
    for input in [
        ViewerImageInput::ActualPixels,
        ViewerImageInput::Zoom {
            factor: 1.234567,
            anchor: None,
        },
        ViewerImageInput::Zoom {
            factor: 0.25,
            anchor: None,
        },
    ] {
        assert!(session.send(ViewerInput::Image {
            frame: displayed.revision,
            input
        }));
        displayed = frame(&session, |frame| frame.revision > displayed.revision);
        let expected_zoom = displayed.image.as_ref().unwrap().zoom;
        assert!(!displayed.image.as_ref().unwrap().fitted);
        for delta in [1, -1] {
            displayed = change_page(&session, &displayed, delta);
            let image = displayed.image.as_ref().unwrap();
            assert_eq!(image.zoom, expected_zoom, "page navigation reset the zoom");
            assert!(!image.fitted, "manual zoom must not become Fit");
            assert_eq!(image.page_count, 2);
            let expected = if image.page == 1 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            let center = view_pixel(&displayed, displayed.width / 2, displayed.height / 2);
            assert_eq!(
                center, expected,
                "the view's centre shows the page turned to"
            );
        }
    }
    assert!(session.send(ViewerInput::Image {
        frame: displayed.revision,
        input: ViewerImageInput::Fit,
    }));
    displayed = frame(&session, |frame| {
        frame.image.as_ref().is_some_and(|image| image.fitted)
    });
    assert_eq!(displayed.image.as_ref().unwrap().zoom, 0.2);
    for delta in [1, -1] {
        displayed = change_page(&session, &displayed, delta);
        let image = displayed.image.as_ref().unwrap();
        assert!(image.fitted);
        assert_eq!(image.zoom, if image.page == 1 { 0.2 } else { 0.16 });
    }
    assert!(session.send(ViewerInput::Resize(ViewerGeometry {
        width: 1200,
        height: 1200,
        scale: 1.0,
    })));
    displayed = frame(&session, |frame| {
        frame
            .image
            .as_ref()
            .is_some_and(|image| image.fitted && image.zoom == 2.0)
    });
    for delta in [1, -1] {
        displayed = change_page(&session, &displayed, delta);
        let image = displayed.image.as_ref().unwrap();
        assert!(image.fitted);
        assert_eq!(image.zoom, if image.page == 1 { 2.0 } else { 1.2 });
    }
    session.close();
}

#[test]
#[ignore = "requires MIDNIGHT_PDF_VIEWER_TEST_APPLICATION with trusted PDF package"]
fn pdf_continuous_composes_native_mixed_pages_and_survives_rapid_camera_replacement() {
    let application =
        std::path::PathBuf::from(std::env::var_os("MIDNIGHT_PDF_VIEWER_TEST_APPLICATION").unwrap());
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (_resident_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-test", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service::new(executor.handle().clone(), resident);
    load_pdf_catalogs(&service, &application);
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("continuous-mixed-pages.pdf");
    std::fs::write(&path, two_page_pdf()).unwrap();
    images::settle_generated_fixture(&path);
    let session = service
        .open(
            launch(files.path(), &["continuous-mixed-pages.pdf"], false),
            appearance(),
            ViewerGeometry {
                width: 400,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    let mut displayed = frame(&session, |frame| frame.image.is_some());
    assert!(session.send(ViewerInput::Image {
        frame: displayed.revision,
        input: ViewerImageInput::ActualPixels,
    }));
    displayed = frame(&session, |frame| frame.revision > displayed.revision);
    assert!(session.send(ViewerInput::DocumentLayout {
        frame: displayed.revision,
        layout: DocumentLayout::Continuous,
    }));
    displayed = frame(&session, |frame| {
        frame
            .image
            .as_ref()
            .is_some_and(|image| image.document_layout == Some(DocumentLayout::Continuous))
    });
    let published = displayed.revision;
    // 1024 logical units total, 200 visible. A 550-unit origin crosses page one and page two.
    assert!(session.send(ViewerInput::Seek(550.0 / 824.0)));
    displayed = frame(&session, |frame| frame.revision > published);
    let image = displayed.image.as_ref().unwrap();
    assert_eq!(image.zoom, 1.0);
    assert_eq!(image.page, 2);
    assert_eq!((image.width, image.height), (1000, 400));
    assert!((image.origin[1] - 550.0).abs() < 0.00001);
    // The view's tiles at that camera: the end of page one, the gap, page two.
    assert_eq!(view_pixel(&displayed, 200, 10), [255, 0, 0, 255]);
    assert_eq!(view_pixel(&displayed, 200, 60), [0, 0, 0, 0]);
    assert_eq!(view_pixel(&displayed, 200, 100), [0, 0, 255, 255]);
    // These replace demand while page crops may be active; they must not hard-cancel the worker.
    for index in 0..32 {
        assert!(session.send(ViewerInput::Seek(f64::from(index) / 32.0)));
    }
    assert!(session.send(ViewerInput::Seek(1.0)));
    displayed = frame(&session, |frame| frame.position == 1.0);
    assert_eq!(displayed.image.as_ref().unwrap().page, 2);
    let current_revision = displayed.revision;
    assert!(session.send(ViewerInput::DocumentLayout {
        frame: published,
        layout: DocumentLayout::Paged,
    }));
    assert!(session.send(ViewerInput::Scroll { x: 0.0, y: -1.0 }));
    displayed = frame(&session, |frame| frame.revision > current_revision);
    assert_eq!(
        displayed.image.as_ref().unwrap().document_layout,
        Some(DocumentLayout::Continuous)
    );
    assert!(session.send(ViewerInput::DocumentLayout {
        frame: displayed.revision,
        layout: DocumentLayout::Paged,
    }));
    displayed = frame(&session, |frame| {
        frame
            .image
            .as_ref()
            .is_some_and(|image| image.document_layout == Some(DocumentLayout::Paged))
    });
    assert_eq!(displayed.image.as_ref().unwrap().zoom, 1.0);
    session.close();
}
