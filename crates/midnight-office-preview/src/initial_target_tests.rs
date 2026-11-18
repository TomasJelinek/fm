//! PRIVATE proposal tests; no canonical registration or test execution.
use super::*;
use crate::SheetId;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counted {
    inner: Arc<dyn OfficeSource>,
    reads: AtomicUsize,
    lengths: AtomicUsize,
}
impl OfficeSource for Counted {
    fn len(&self) -> u64 {
        self.lengths.fetch_add(1, Ordering::Relaxed);
        self.inner.len()
    }
    fn read_at(&self, offset: u64, output: &mut [u8], cancel: &Cancellation) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.read_at(offset, output, cancel)
    }
}
impl Counted {
    fn new(inner: Arc<dyn OfficeSource>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            reads: AtomicUsize::new(0),
            lengths: AtomicUsize::new(0),
        })
    }
    fn calls(&self) -> (usize, usize) {
        (
            self.reads.load(Ordering::Relaxed),
            self.lengths.load(Ordering::Relaxed),
        )
    }
}
fn xlsx(empty: bool) -> Arc<dyn OfficeSource> {
    let sheets = if empty {
        String::new()
    } else {
        r#"<w:sheet name="Initial sheet" sheetId="91" r:id="first"/>"#.into()
    };
    zip(&[
        ("[Content_Types].xml",r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/book.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#.into()),
        ("_rels/.rels",r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="doc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/book.xml"/></Relationships>"#.into()),
        ("xl/book.xml",format!(r#"<w:workbook xmlns:w="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:sheets>{sheets}</w:sheets></w:workbook>"#)),
        ("xl/_rels/book.xml.rels",r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="first" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="sheet.xml"/></Relationships>"#.into()),
        ("xl/sheet.xml",r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>First XLSX cell</t></is></c></row></sheetData></worksheet>"#.into()),
    ])
}
fn empty_pptx() -> Arc<dyn OfficeSource> {
    zip(&[
        ("[Content_Types].xml",r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/main.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#.into()),
        ("_rels/.rels",r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="doc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/main.xml"/></Relationships>"#.into()),
        ("ppt/main.xml",r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst/></p:presentation>"#.into()),
        ("ppt/_rels/main.xml.rels",r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#.into()),
    ])
}

#[test]
fn initial_target_is_metadata_only_and_first_render_contains_real_contents_for_all_six_formats() {
    let doc = OfficeTarget::DocumentWindow {
        document: DocumentId(0),
        block: 0,
    };
    let sheet = OfficeTarget::SheetFirst { sheet: SheetId(0) };
    let slide = OfficeTarget::SlideWindow {
        slide: SlideId(0),
        block: 0,
    };
    for (source, format, expected, text) in [
        (
            docx(
                "<different:p><different:r><different:t>First DOCX text</different:t></different:r></different:p>",
                false,
            ),
            OfficeFormat::Docx,
            doc,
            "First DOCX text",
        ),
        (
            odf("<t:p>First ODT text</t:p>", "text"),
            OfficeFormat::Odt,
            doc,
            "First ODT text",
        ),
        (xlsx(false), OfficeFormat::Xlsx, sheet, "First XLSX cell"),
        (
            odf(
                r#"<g:table g:name="First"><g:table-row><g:table-cell><t:p>First ODS cell</t:p></g:table-cell></g:table-row></g:table>"#,
                "spreadsheet",
            ),
            OfficeFormat::Ods,
            sheet,
            "First ODS cell",
        ),
        (
            pptx(
                "",
                "<q:sp><q:txBody><a:p><a:r><a:t>First PPTX slide</a:t></a:r></a:p></q:txBody></q:sp>",
                false,
            ),
            OfficeFormat::Pptx,
            slide,
            "First PPTX slide",
        ),
        (
            odf(
                "<d:page><d:frame><d:text-box><t:p>First ODP slide</t:p></d:text-box></d:frame></d:page>",
                "presentation",
            ),
            OfficeFormat::Odp,
            slide,
            "First ODP slide",
        ),
    ] {
        let counted = Counted::new(source);
        let mut session = open(counted.clone(), format);
        let before = counted.calls();
        for _ in 0..3 {
            assert_eq!(session.initial_target(), expected);
        }
        assert_eq!(session.parse_target(&expected.path()).unwrap(), expected);
        assert_eq!(
            counted.calls(),
            before,
            "initial target and validation cannot read source"
        );
        let page = session
            .render(session.initial_target(), &Cancellation::default())
            .unwrap();
        assert_eq!(
            page.generation, 1,
            "metadata helper cannot advance generation"
        );
        assert_eq!(page.target, expected);
        assert!(
            String::from_utf8(page.bytes.to_vec())
                .unwrap()
                .contains(text)
        );
    }
}

#[test]
fn only_absent_family_contents_fall_back_to_catalog() {
    for (source, format) in [
        (xlsx(true), OfficeFormat::Xlsx),
        (odf("", "spreadsheet"), OfficeFormat::Ods),
        (empty_pptx(), OfficeFormat::Pptx),
        (odf("", "presentation"), OfficeFormat::Odp),
    ] {
        let counted = Counted::new(source);
        let session = open(counted.clone(), format);
        let before = counted.calls();
        assert_eq!(session.initial_target(), OfficeTarget::Catalog);
        assert_eq!(counted.calls(), before);
    }
    for (source, format) in [
        (docx("", false), OfficeFormat::Docx),
        (odf("", "text"), OfficeFormat::Odt),
        (pptx("", "", false), OfficeFormat::Pptx),
        (odf("<d:page/>", "presentation"), OfficeFormat::Odp),
    ] {
        let mut session = open(source, format);
        let target = session.initial_target();
        assert_ne!(target, OfficeTarget::Catalog);
        let page = session.render(target, &Cancellation::default()).unwrap();
        assert!(
            String::from_utf8(page.bytes.to_vec())
                .unwrap()
                .contains("Empty content window")
        );
    }
}

#[test]
fn unsupported_initial_contents_surface_without_catalog_fallback() {
    let mut session = open(
        docx(
            "<different:p><different:r><different:drawing/></different:r></different:p>",
            false,
        ),
        OfficeFormat::Docx,
    );
    let target = session.initial_target();
    assert!(matches!(target, OfficeTarget::DocumentWindow { .. }));
    assert!(matches!(
        session.render(target, &Cancellation::default()),
        Err(OfficeError::Unsupported(_))
    ));
    assert_eq!(session.initial_target(), target);
    assert_eq!(
        session
            .render(OfficeTarget::Catalog, &Cancellation::default())
            .unwrap()
            .generation,
        1
    );
}

#[test]
fn new_paths_are_canonical_and_direct_wrong_family_targets_do_not_read() {
    for target in [
        OfficeTarget::DocumentWindow {
            document: DocumentId(0),
            block: 1_048_575,
        },
        OfficeTarget::SlideWindow {
            slide: SlideId(4095),
            block: 64,
        },
    ] {
        assert_eq!(OfficeTarget::parse(&target.path()).unwrap(), target);
    }
    for path in [
        "/office/v1/document/00/block/0.html",
        "/office/v1/document/0/block/01.html",
        "/office/v1/document/0/block/1048576.html",
        "/office/v1/slide/0/block/0.html?x=1",
        "/office/v1/slide/0/block/0.html#x",
        "/office/v1/slide/%30/block/0.html",
        "/office/v1/slide/0/block/-1.html",
        "/office/v1/slide/0/block/0.html/",
    ] {
        assert!(
            matches!(OfficeTarget::parse(path), Err(OfficeError::InvalidTarget)),
            "{path}"
        );
    }
    let counted = Counted::new(docx("", false));
    let mut session = open(counted.clone(), OfficeFormat::Docx);
    let before = counted.calls();
    for target in [
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
        OfficeTarget::SlideWindow {
            slide: SlideId(0),
            block: 0,
        },
        OfficeTarget::DocumentWindow {
            document: DocumentId(1),
            block: 0,
        },
    ] {
        assert!(matches!(
            session.parse_target(&target.path()),
            Err(OfficeError::InvalidTarget)
        ));
        assert!(matches!(
            session.render(target, &Cancellation::default()),
            Err(OfficeError::InvalidTarget)
        ));
    }
    assert_eq!(counted.calls(), before);
}

#[test]
fn small_member_reads_enforce_size_and_keep_the_existing_operation_budget() {
    let source = Counted::new(zip(&[("small", "abcd".into())]));
    let mut package =
        crate::package::Package::open(source.clone(), &Cancellation::default()).unwrap();
    assert_eq!(package.read_small("small", 4).unwrap(), b"abcd");
    assert!(matches!(
        package.read_small("small", 3),
        Err(OfficeError::LimitExceeded(_))
    ));
    let before = source.calls();
    assert!(matches!(
        package.read_small("small", 65_537),
        Err(OfficeError::LimitExceeded(_))
    ));
    assert_eq!(source.calls(), before);
    assert!(
        package
            .work
            .reserve(crate::source::MAX_WORK as usize)
            .unwrap()
            > 0
    );
    assert!(
        matches!(
            package.read_small("small", 4),
            Err(OfficeError::LimitExceeded(_))
        ),
        "helper must not reset the operation budget"
    );
}
