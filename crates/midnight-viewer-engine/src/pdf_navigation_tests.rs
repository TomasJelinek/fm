//! Host navigation acceptance using the separately built, sandboxed PDF plugin.
use super::*;
use midnight_core::ViewerImageInput;

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
    let service = Service::new(executor.handle().clone());
    *service.runtime.lock().unwrap() = Some(Arc::new(
        Runtime::discover(application.parent().unwrap()).unwrap(),
    ));
    let providers = midnight_image_viewer_host::discover(&application).unwrap();
    assert!(
        providers
            .iter()
            .any(|provider| provider.id == "midnight.viewer.pdf")
    );
    *service.images.lock().unwrap() = Some(providers.into());
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
            let center =
                ((displayed.height / 2 * displayed.width + displayed.width / 2) * 4) as usize;
            assert_eq!(&displayed.pixels[center..center + 4], &expected);
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
