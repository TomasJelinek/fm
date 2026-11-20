//! Host boundary acceptance; renderer behavior and PDF fixtures belong to the independent plugin.
use midnight_image_viewer_host::{ImageSession, Source, Viewport};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Captured {
    bytes: Vec<u8>,
    changed: AtomicBool,
    reads: AtomicUsize,
}
impl Source for Captured {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn version(&self) -> Vec<u8> {
        vec![u8::from(self.changed.load(Ordering::Acquire))]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self.bytes[offset as usize..offset as usize + length as usize].to_vec())
    }
}

fn source() -> Arc<Captured> {
    // Authored source; Hayro's cross-reference recovery accepts this minimal PDF.
    let bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 25 >>\nstream\n1 0 0 rg 0 0 20 20 re f\nendstream\nendobj\n5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 30 20] /Contents 6 0 R >>\nendobj\n6 0 obj\n<< /Length 25 >>\nstream\n0 0 1 rg 0 0 30 20 re f\nendstream\nendobj\ntrailer\n<< /Root 1 0 R /Size 7 >>\n%%EOF\n".to_vec();
    Arc::new(Captured {
        bytes,
        changed: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    })
}

fn provider() -> midnight_image_viewer_host::Provider {
    let path = PathBuf::from(
        std::env::var_os("MIDNIGHT_PDF_VIEWER_TEST_APPLICATION")
            .expect("Explicit private PDF test installation required"),
    );
    let discovery = midnight_image_viewer_host::discover_with_diagnostics(&path).unwrap();
    assert!(
        discovery.diagnostics.is_empty(),
        "{:?}",
        discovery.diagnostics
    );
    discovery
        .providers
        .into_iter()
        .find(|provider| provider.id == "midnight.viewer.pdf")
        .unwrap()
}

fn viewport() -> Viewport {
    Viewport {
        width: 8,
        height: 8,
        scale: 2.0,
        origin_x: 0.0,
        origin_y: 0.0,
    }
}

#[test]
#[ignore = "Requires MIDNIGHT_PDF_VIEWER_TEST_APPLICATION in a trusted private installation"]
fn sandboxed_pdf_pages_use_retained_source_and_reject_revision_change() {
    let source = source();
    let cancel = AtomicBool::new(false);
    let mut session = ImageSession::open(
        &provider(),
        source.clone(),
        &cancel,
        Duration::from_secs(15),
    )
    .unwrap();
    assert!(session.is_document());
    assert_eq!(session.page_count(), 2);
    assert_eq!(session.page_metadata(1).unwrap().width, 30);
    let reads = source.reads.load(Ordering::Relaxed);
    assert!(reads > 0);
    for (page, expected) in [
        (0, [255, 0, 0, 255]),
        (1, [0, 0, 255, 255]),
        (0, [255, 0, 0, 255]),
    ] {
        let frame = session
            .render_page(page, viewport(), &cancel, Duration::from_secs(10))
            .unwrap();
        assert!(
            frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == expected)
        );
    }
    assert_eq!(source.reads.load(Ordering::Relaxed), reads);
    source.changed.store(true, Ordering::Release);
    assert!(
        session
            .render_page(1, viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
    source.changed.store(false, Ordering::Release);
    assert!(
        session
            .render_page(0, viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
    session.close_checked().unwrap();
}

/// One 20 x 20 pt page with `content`, with a complete cross-reference table.
fn one_page(content: &str) -> Arc<Captured> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Contents 4 0 R >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    Arc::new(Captured {
        bytes,
        changed: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    })
}

#[test]
#[ignore = "Requires MIDNIGHT_PDF_VIEWER_TEST_APPLICATION in a trusted private installation"]
fn refusals_carry_the_parser_and_renderer_reasons() {
    let provider = provider();
    let cancel = AtomicBool::new(false);
    let error = ImageSession::open(
        &provider,
        Arc::new(Captured {
            bytes: b"not a PDF".to_vec(),
            changed: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
        }),
        &cancel,
        Duration::from_secs(15),
    )
    .err()
    .expect("text is not a document");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(
        error.to_string(),
        "This document is damaged or incomplete: the PDF structure is invalid or incomplete"
    );
    // An undecodable inline image opens, but its page cannot be drawn.
    let mut session = ImageSession::open(
        &provider,
        one_page("q 20 0 0 20 0 0 cm BI /W 4 /H 4 /BPC 8 /CS /RGB /F /DCT ID notajpeg EI Q"),
        &cancel,
        Duration::from_secs(15),
    )
    .unwrap();
    let error = session
        .render_page(0, viewport(), &cancel, Duration::from_secs(15))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "This page could not be drawn: the page contains content this viewer cannot draw"
    );
    assert!(
        session
            .render_page(0, viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
}

#[test]
#[ignore = "Requires MIDNIGHT_PDF_VIEWER_TEST_APPLICATION in a trusted private installation"]
fn cancelled_pdf_session_cannot_be_reused() {
    let cancel = AtomicBool::new(false);
    let mut session =
        ImageSession::open(&provider(), source(), &cancel, Duration::from_secs(15)).unwrap();
    cancel.store(true, Ordering::Release);
    let error = session
        .render_page(1, viewport(), &cancel, Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    cancel.store(false, Ordering::Release);
    assert!(
        session
            .render_page(0, viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
}
