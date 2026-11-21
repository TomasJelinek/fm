//! Five-format fixtures; register only after the shared service API freeze.
#[path = "initial_target_tests.rs"]
mod initial_target_tests;
use crate::{
    Cancellation, DocumentId, OfficeError, OfficeFormat, OfficeSession, OfficeSource, OfficeTarget,
    SlideId,
};
use std::{
    io::{self, Cursor, Write},
    sync::Arc,
};
use zip::{ZipWriter, write::SimpleFileOptions};

struct Source(Vec<u8>);
impl OfficeSource for Source {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8], _: &Cancellation) -> io::Result<usize> {
        let bytes = self
            .0
            .get(offset as usize..)
            .ok_or_else(|| io::Error::other("offset"))?;
        let length = bytes.len().min(output.len());
        output[..length].copy_from_slice(&bytes[..length]);
        Ok(length)
    }
}
fn zip(parts: &[(&str, String)]) -> Arc<dyn OfficeSource> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in parts {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    Arc::new(Source(writer.finish().unwrap().into_inner()))
}
fn docx(body: &str, strict: bool) -> Arc<dyn OfficeSource> {
    let word = if strict {
        "http://purl.oclc.org/ooxml/wordprocessingml/main"
    } else {
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    };
    let relationships = if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    };
    zip(&[
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/main.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.into()),
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{relationships}/officeDocument" Target="/word/main.xml"/></Relationships>"#)),
        ("word/main.xml", format!(r#"<different:document xmlns:different="{word}"><different:body>{body}</different:body></different:document>"#)),
    ])
}
fn odf(body: &str, kind: &str) -> Arc<dyn OfficeSource> {
    zip(&[
        (
            "mimetype",
            format!("application/vnd.oasis.opendocument.{kind}"),
        ),
        (
            "META-INF/manifest.xml",
            format!(
                r#"<m:manifest xmlns:m="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" m:version="1.3"><m:file-entry m:full-path="/" m:media-type="application/vnd.oasis.opendocument.{kind}"/><m:file-entry m:full-path="content.xml" m:media-type="text/xml"/></m:manifest>"#
            ),
        ),
        (
            "content.xml",
            format!(
                r#"<o:document-content xmlns:o="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:t="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:g="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:d="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" o:version="1.3"><o:body><o:{kind}>{body}</o:{kind}></o:body></o:document-content>"#
            ),
        ),
    ])
}
fn open(source: Arc<dyn OfficeSource>, format: OfficeFormat) -> OfficeSession {
    OfficeSession::open(source, format, &Cancellation::default()).unwrap()
}
fn render(session: &mut OfficeSession, block: u32) -> String {
    let target = OfficeTarget::DocumentWindow {
        document: DocumentId(0),
        block,
    };
    assert_eq!(session.parse_target(&target.path()).unwrap(), target);
    String::from_utf8(
        session
            .render(target, &Cancellation::default())
            .unwrap()
            .bytes
            .to_vec(),
    )
    .unwrap()
}

#[test]
fn docx_transitional_and_strict_prefixes_preserve_paragraph_table_order_and_escape() {
    let body = r#"<different:p><different:r><different:t>Žluťoučký &lt;script&gt;&amp;</different:t></different:r></different:p><different:tbl><different:tr><different:tc><different:p><different:r><different:t>Left cell</different:t></different:r></different:p></different:tc><different:tc><different:p><different:r><different:t>Right cell</different:t></different:r></different:p></different:tc></different:tr></different:tbl><different:p><different:r><different:t>Last paragraph</different:t></different:r></different:p>"#;
    for strict in [false, true] {
        let mut session = open(docx(body, strict), OfficeFormat::Docx);
        assert!(session.catalog().sheets.is_empty());
        assert_eq!(
            session.catalog().document.as_ref().unwrap().id,
            DocumentId(0)
        );
        let html = render(&mut session, 0);
        assert!(html.contains("Žluťoučký &lt;script&gt;&amp;"));
        assert!(!html.contains("<script>"));
        assert!(html.find("Left cell").unwrap() < html.find("Right cell").unwrap());
        assert!(html.find("Right cell").unwrap() < html.find("Last paragraph").unwrap());
        assert!(html.contains("<table>"));
    }
}

#[test]
fn document_windows_do_not_repeat_or_lose_paragraphs() {
    let body = (0..130).map(|i|format!("<different:p><different:r><different:t>Paragraph-{i:03}</different:t></different:r></different:p>")).collect::<String>();
    let mut session = open(docx(&body, false), OfficeFormat::Docx);
    let first = render(&mut session, 0);
    assert!(first.contains("Paragraph-000") && first.contains("Paragraph-063"));
    assert!(!first.contains("Paragraph-064"));
    assert!(first.contains("/office/v1/document/0/block/64.html"));
    let second = render(&mut session, 64);
    assert!(second.contains("Paragraph-064") && second.contains("Paragraph-127"));
    assert!(!second.contains("Paragraph-063") && !second.contains("Paragraph-128"));
}

#[test]
fn odt_prefixes_lists_spaces_and_tables_keep_reading_order() {
    let body = r#"<t:h t:outline-level="1">Heading</t:h><t:list><t:list-item><t:p>First<t:s t:c="3"/>item</t:p></t:list-item></t:list><g:table g:name="Values"><g:table-row><g:table-cell><t:p>A &amp; B</t:p></g:table-cell><g:table-cell><t:p>Český text</t:p></g:table-cell></g:table-row></g:table>"#;
    let mut session = open(odf(body, "text"), OfficeFormat::Odt);
    let html = render(&mut session, 0);
    assert!(html.contains("First   item"));
    assert!(html.contains("A &amp; B") && html.contains("Český text"));
    assert!(html.find("Heading").unwrap() < html.find("First   item").unwrap());
    assert!(html.find("First   item").unwrap() < html.find("A &amp; B").unwrap());
}

const OBJECTS_ONLY: &str = "contains only pictures, charts or other objects that the preview does \
     not show";

#[test]
fn empty_and_object_only_documents_are_not_confused() {
    let mut empty = open(docx("", false), OfficeFormat::Docx);
    let page = render(&mut empty, 0);
    assert!(page.contains("Empty") && !page.contains(OBJECTS_ONLY));
    let mut objects = open(
        docx(
            "<different:p><different:r><different:drawing/></different:r></different:p>",
            false,
        ),
        OfficeFormat::Docx,
    );
    let page = render(&mut objects, 0);
    assert!(page.contains(&format!("This part of the document {OBJECTS_ONLY}.")));
    assert!(!page.contains("Empty content window"));
}

#[test]
fn document_text_is_labelled_when_truncated_and_cancellation_has_no_html() {
    let text = "é".repeat(20_000);
    let mut session = open(
        docx(
            &format!(
                "<different:p><different:r><different:t>{text}</different:t></different:r></different:p>"
            ),
            false,
        ),
        OfficeFormat::Docx,
    );
    let html = render(&mut session, 0);
    assert!(html.contains("[truncated]"));
    assert!(html.len() < 40_000);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        session.render(
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: 0
            },
            &cancel
        ),
        Err(OfficeError::Cancelled)
    ));
}

#[test]
fn document_targets_reject_wrong_family_membership_and_noncanonical_paths() {
    let session = open(docx("", false), OfficeFormat::Docx);
    for path in [
        "/office/v1/document/1/block/0.html",
        "/office/v1/document/0/block/00.html",
        "/office/v1/document/0/block/1048576.html",
        "/office/v1/sheet/0/first.html",
        "/office/v1/slide/0/block/0.html",
    ] {
        assert!(
            matches!(session.parse_target(path), Err(OfficeError::InvalidTarget)),
            "{path}"
        );
    }
}

fn pptx(first: &str, second: &str, strict: bool) -> Arc<dyn OfficeSource> {
    let presentation = if strict {
        "http://purl.oclc.org/ooxml/presentationml/main"
    } else {
        "http://schemas.openxmlformats.org/presentationml/2006/main"
    };
    let drawing = if strict {
        "http://purl.oclc.org/ooxml/drawingml/main"
    } else {
        "http://schemas.openxmlformats.org/drawingml/2006/main"
    };
    let relationships = if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    };
    zip(&[
        ("[Content_Types].xml",r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/main.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#.into()),
        ("_rels/.rels",format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{relationships}/officeDocument" Target="ppt/main.xml"/></Relationships>"#)),
        ("ppt/main.xml",format!(r#"<q:presentation xmlns:q="{presentation}" xmlns:r="{relationships}"><q:sldIdLst><q:sldId id="257" r:id="second"/><q:sldId id="256" r:id="first"/></q:sldIdLst></q:presentation>"#)),
        ("ppt/_rels/main.xml.rels",format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="first" Type="{relationships}/slide" Target="slides/a.xml"/><Relationship Id="second" Type="{relationships}/slide" Target="/ppt/slides/b.xml"/></Relationships>"#)),
        ("ppt/slides/a.xml",format!(r#"<q:sld xmlns:q="{presentation}" xmlns:a="{drawing}"><q:cSld><q:spTree>{first}</q:spTree></q:cSld></q:sld>"#)),
        ("ppt/slides/b.xml",format!(r#"<q:sld xmlns:q="{presentation}" xmlns:a="{drawing}"><q:cSld><q:spTree>{second}</q:spTree></q:cSld></q:sld>"#)),
    ])
}
fn slide(session: &mut OfficeSession, id: u16, block: u32) -> String {
    let target = OfficeTarget::SlideWindow {
        slide: SlideId(id),
        block,
    };
    assert_eq!(session.parse_target(&target.path()).unwrap(), target);
    String::from_utf8(
        session
            .render(target, &Cancellation::default())
            .unwrap()
            .bytes
            .to_vec(),
    )
    .unwrap()
}

#[test]
fn pptx_relationship_order_strict_prefixes_and_table_text_are_readable() {
    let first = "<q:sp><q:txBody><a:p><a:r><a:t>Slide A</a:t></a:r></a:p></q:txBody></q:sp>";
    let second = r#"<q:sp><q:txBody><a:p><a:r><a:t>Slide B &lt;safe&gt;</a:t></a:r></a:p></q:txBody></q:sp><q:graphicFrame><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Table value</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></q:graphicFrame>"#;
    for strict in [false, true] {
        let mut session = open(pptx(first, second, strict), OfficeFormat::Pptx);
        assert_eq!(session.catalog().slides.len(), 2);
        let page = slide(&mut session, 0, 0);
        assert!(page.contains("Slide B &lt;safe&gt;") && page.contains("Table value"));
        assert!(!page.contains("Slide A"));
        assert!(slide(&mut session, 1, 0).contains("Slide A"));
        assert!(
            session
                .parse_target("/office/v1/slide/2/block/0.html")
                .is_err()
        );
    }
}

#[test]
fn odp_page_order_and_named_slides_preserve_text_tables_and_empty_state() {
    let body = r#"<d:page d:name="First &amp; safe"><d:frame><d:text-box><t:p>ODP text</t:p></d:text-box></d:frame><d:frame><g:table g:name="Data"><g:table-row><g:table-cell><t:p>ODP cell</t:p></g:table-cell></g:table-row></g:table></d:frame></d:page><d:page d:name="Empty"/>"#;
    let mut session = open(odf(body, "presentation"), OfficeFormat::Odp);
    assert_eq!(session.catalog().slides[0].name, "Slide 1");
    let first = slide(&mut session, 0, 0);
    assert!(first.contains("ODP text") && first.contains("ODP cell"));
    assert!(slide(&mut session, 1, 0).contains("Empty content"));
}

#[test]
fn slide_text_windows_continue_and_image_only_slide_has_a_notice() {
    let paragraphs = (0..70)
        .map(|i| format!("<a:p><a:r><a:t>Line-{i:03}</a:t></a:r></a:p>"))
        .collect::<String>();
    let mut session = open(
        pptx(
            "<q:pic/>",
            &format!("<q:sp><q:txBody>{paragraphs}</q:txBody></q:sp>"),
            false,
        ),
        OfficeFormat::Pptx,
    );
    let first = slide(&mut session, 0, 0);
    assert!(first.contains("Line-063") && !first.contains("Line-064"));
    assert!(first.contains("/office/v1/slide/0/block/64.html"));
    assert!(slide(&mut session, 0, 64).contains("Line-069"));
    let picture = slide(&mut session, 1, 0);
    assert!(picture.contains(&format!("This slide {OBJECTS_ONLY}.")));
    assert!(!picture.contains("Empty content window"));
}

#[test]
fn table_rows_continue_across_document_windows_and_extra_columns_are_labelled() {
    let rows=(0..70).map(|i|format!("<different:tr><different:tc><different:p><different:r><different:t>Row-{i:03}</different:t></different:r></different:p></different:tc></different:tr>")).collect::<String>();
    let mut session = open(
        docx(&format!("<different:tbl>{rows}</different:tbl>"), false),
        OfficeFormat::Docx,
    );
    let first = render(&mut session, 0);
    assert!(first.contains("Row-063") && !first.contains("Row-064"));
    let second = render(&mut session, 64);
    assert!(second.contains("Row-064") && second.contains("Row-069"));
    let columns=(0..40).map(|i|format!("<different:tc><different:p><different:r><different:t>Column-{i:02}</different:t></different:r></different:p></different:tc>")).collect::<String>();
    let mut session = open(
        docx(
            &format!("<different:tbl><different:tr>{columns}</different:tr></different:tbl>"),
            false,
        ),
        OfficeFormat::Docx,
    );
    let page = render(&mut session, 0);
    assert!(page.contains("Column-31") && !page.contains("Column-32"));
    assert!(page.contains("[additional columns omitted]"));
}

#[test]
fn odt_huge_repeated_spaces_are_bounded_and_rows_skip_arithmetically() {
    let body = r#"<t:p>A<t:s t:c="4294967295"/>Z</t:p><g:table g:name="Repeated"><g:table-row g:number-rows-repeated="1000000"><g:table-cell><t:p>Repeated row</t:p></g:table-cell></g:table-row></g:table>"#;
    let mut session = open(odf(body, "text"), OfficeFormat::Odt);
    let first = render(&mut session, 0);
    assert!(first.contains("[truncated]"));
    assert!(first.len() < 50_000);
    let far = render(&mut session, 999_999);
    assert_eq!(far.matches("Repeated row").count(), 2);
    assert!(far.len() < 10_000);
}

#[test]
fn odf_encrypted_manifest_is_rejected_before_any_preview() {
    let source=zip(&[
        ("mimetype","application/vnd.oasis.opendocument.text".into()),
        ("META-INF/manifest.xml",r#"<m:manifest xmlns:m="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><m:file-entry m:full-path="/" m:media-type="application/vnd.oasis.opendocument.text"/><m:file-entry m:full-path="content.xml" m:media-type="text/xml"><m:encryption-data/></m:file-entry></m:manifest>"#.into()),
        ("content.xml","this must never become successful blank HTML".into()),
    ]);
    assert!(matches!(
        OfficeSession::open(source, OfficeFormat::Odt, &Cancellation::default()),
        Err(OfficeError::Encrypted)
    ));
}

#[test]
fn wrong_namespace_content_cannot_impersonate_word_text_and_mixed_objects_are_labelled() {
    let body = r#"<evil:p xmlns:evil="urn:untrusted"><evil:r><evil:t>Forged contents</evil:t></evil:r></evil:p><different:p><different:r><different:t>Real contents</different:t><different:drawing/></different:r></different:p>"#;
    let mut session = open(docx(body, false), OfficeFormat::Docx);
    let page = render(&mut session, 0);
    assert!(!page.contains("Forged contents"));
    assert!(page.contains("Real contents") && page.contains("omitted"));
}

#[test]
fn malformed_table_paragraph_outside_a_cell_is_not_successful_blank_content() {
    let body = "<different:tbl><different:p><different:r><different:t>Lost content</different:t></different:r></different:p></different:tbl>";
    let mut session = open(docx(body, false), OfficeFormat::Docx);
    assert!(matches!(
        session.render(
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: 0
            },
            &Cancellation::default()
        ),
        Err(OfficeError::Malformed(_))
    ));
}

#[test]
fn early_text_and_escaped_html_windows_resume_at_first_omitted_block() {
    for (raw, label) in [
        ("x".repeat(32_000), "text"),
        ("&quot;".repeat(10_000), "escaping"),
    ] {
        let body=(0..70).map(|i|format!("<different:p><different:r><different:t>Block-{i:03}:{raw}</different:t></different:r></different:p>")).collect::<String>();
        let mut session = open(docx(&body, false), OfficeFormat::Docx);
        let first = render(&mut session, 0);
        let count = first.matches("Block-").count();
        assert!(count > 0 && count < 64, "{label}: must end early");
        assert!(first.contains(&format!("/office/v1/document/0/block/{count}.html")));
        let next = render(&mut session, count as u32);
        assert!(
            next.contains(&format!("Block-{count:03}:")),
            "{label}: first omitted block lost"
        );
        assert!(first.len() <= 1024 * 1024 && next.len() <= 1024 * 1024);
    }
}

#[test]
fn odt_zero_spaces_are_valid_and_slide_navigation_is_numeric() {
    let mut session = open(
        odf("<t:p>A<t:s t:c=\"0\"/>B</t:p>", "text"),
        OfficeFormat::Odt,
    );
    assert!(render(&mut session, 0).contains("AB"));
    let mut session = open(pptx("", "", false), OfficeFormat::Pptx);
    assert!(slide(&mut session, 0, 0).contains("/office/v1/slide/1/block/0.html"));
    assert!(slide(&mut session, 1, 0).contains("/office/v1/slide/0/block/0.html"));
}

#[test]
fn window_past_actual_content_is_empty_even_when_earlier_objects_were_unsupported() {
    let mut session = open(
        docx(
            "<different:p><different:r><different:drawing/></different:r></different:p>",
            false,
        ),
        OfficeFormat::Docx,
    );
    let page = render(&mut session, 99);
    assert!(page.contains("Empty content window"));
    assert!(!page.contains("Next contents"));
}

#[test]
fn table_newlines_do_not_disguise_unsupported_only_objects_as_readable_contents() {
    let body = "<different:tbl><different:tr><different:tc><different:p><different:r><different:drawing/></different:r></different:p></different:tc></different:tr></different:tbl>";
    let mut session = open(docx(body, false), OfficeFormat::Docx);
    let page = render(&mut session, 0);
    assert!(page.contains(OBJECTS_ONLY) && !page.contains("Empty content window"));
}

#[test]
fn odt_table_stored_scalar_survives_without_a_display_paragraph() {
    let body = r#"<g:table g:name="Numbers"><g:table-row><g:table-cell o:value-type="float" o:value="42"/><g:table-cell o:value-type="float" o:value="99"><t:p>Formatted value</t:p></g:table-cell></g:table-row></g:table>"#;
    let mut session = open(odf(body, "text"), OfficeFormat::Odt);
    let page = render(&mut session, 0);
    assert!(page.contains("42") && page.contains("Formatted value"));
    assert!(!page.contains(">99"));
}

#[test]
fn odp_long_slide_repeated_rows_continue_without_expanding_the_prefix() {
    let body = r#"<d:page d:name="Long"><d:frame><g:table g:name="Rows"><g:table-row g:number-rows-repeated="1000000"><g:table-cell g:number-columns-repeated="16384"><t:p>ODP repeated</t:p></g:table-cell></g:table-row></g:table></d:frame></d:page>"#;
    let mut session = open(odf(body, "presentation"), OfficeFormat::Odp);
    let first = slide(&mut session, 0, 0);
    assert!(first.contains("/office/v1/slide/0/block/64.html"));
    assert!(first.contains("additional columns omitted"));
    let last = slide(&mut session, 0, 999_999);
    assert_eq!(last.matches("ODP repeated").count(), 32);
    assert!(!last.contains("Next contents"));
    assert!(last.len() < 10_000);
}

#[test]
fn cancelled_document_and_slide_generations_publish_nothing_and_do_not_advance_id() {
    let document = OfficeTarget::DocumentWindow {
        document: DocumentId(0),
        block: 0,
    };
    let slide = OfficeTarget::SlideWindow {
        slide: SlideId(0),
        block: 0,
    };
    for (source, format, target) in [
        (docx("", false), OfficeFormat::Docx, document),
        (odf("", "text"), OfficeFormat::Odt, document),
        (pptx("", "", false), OfficeFormat::Pptx, slide),
        (odf("<d:page/>", "presentation"), OfficeFormat::Odp, slide),
    ] {
        let mut session = open(source, format);
        assert_eq!(
            session
                .render(target, &Cancellation::default())
                .unwrap()
                .generation,
            1
        );
        let cancelled = Cancellation::default();
        cancelled.cancel();
        assert!(matches!(
            session.render(target, &cancelled),
            Err(OfficeError::Cancelled)
        ));
        assert_eq!(
            session
                .render(target, &Cancellation::default())
                .unwrap()
                .generation,
            2
        );
    }
}

#[test]
fn complete_table_row_text_is_bounded_including_all_displayed_cells() {
    let content = "é".repeat(10_000);
    let cells=(0..32).map(|_|format!("<different:tc><different:p><different:r><different:t>{content}</different:t></different:r></different:p></different:tc>")).collect::<String>();
    let mut session = open(
        docx(
            &format!("<different:tbl><different:tr>{cells}</different:tr></different:tbl>"),
            false,
        ),
        OfficeFormat::Docx,
    );
    let page = render(&mut session, 0);
    assert!(page.contains("[truncated]"));
    assert!(page.contains("additional columns omitted"));
    assert!(
        page.len() < 40_000,
        "row must not retain 32 full-length cells"
    );
}

#[test]
fn odf_document_and_slide_column_repeats_reject_coordinate_overflow() {
    let table = r#"<g:table g:name="Bad"><g:table-row><g:table-cell g:number-columns-repeated="16384"/><g:table-cell/></g:table-row></g:table>"#;
    let mut document = open(odf(table, "text"), OfficeFormat::Odt);
    assert!(matches!(
        document.render(
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: 0
            },
            &Cancellation::default()
        ),
        Err(OfficeError::LimitExceeded(_))
    ));
    let mut presentation = open(
        odf(
            &format!("<d:page><d:frame>{table}</d:frame></d:page>"),
            "presentation",
        ),
        OfficeFormat::Odp,
    );
    assert!(matches!(
        presentation.render(
            OfficeTarget::SlideWindow {
                slide: SlideId(0),
                block: 0
            },
            &Cancellation::default()
        ),
        Err(OfficeError::LimitExceeded(_))
    ));
}

#[test]
fn repeated_odf_rows_keep_omissions_for_the_requested_intersection() {
    for text in ["", "Visible repeated content"] {
        let table = format!(
            r#"<g:table g:name="Repeated"><g:table-row g:number-rows-repeated="1000"><g:table-cell><t:p>{text}<d:frame><d:image/></d:frame></t:p></g:table-cell></g:table-row></g:table>"#
        );
        for (source, format, target) in [
            (
                odf(&table, "text"),
                OfficeFormat::Odt,
                OfficeTarget::DocumentWindow {
                    document: DocumentId(0),
                    block: 500,
                },
            ),
            (
                odf(
                    &format!("<d:page><d:frame>{table}</d:frame></d:page>"),
                    "presentation",
                ),
                OfficeFormat::Odp,
                OfficeTarget::SlideWindow {
                    slide: SlideId(0),
                    block: 500,
                },
            ),
        ] {
            let mut session = open(source, format);
            let result = session.render(target, &Cancellation::default());
            let page = String::from_utf8(result.unwrap().bytes.to_vec()).unwrap();
            if text.is_empty() {
                assert!(page.contains(OBJECTS_ONLY) && !page.contains("Empty content window"));
            } else {
                assert_eq!(page.matches(text).count(), 64);
                assert!(page.contains("unsupported layout contents are omitted"));
                assert!(page.contains("/block/564.html"));
            }
        }
    }
}

#[test]
fn rejected_next_docx_block_cannot_contaminate_the_admitted_empty_window() {
    let space = " ".repeat(32_756);
    let mut body = format!(
        "<different:p><different:r><different:t>{space}</different:t></different:r></different:p>"
    )
    .repeat(16);
    body.push_str(&format!("<different:p><different:r><different:t>{}</different:t><different:drawing/></different:r></different:p>","Next-visible".repeat(100)));
    let mut session = open(docx(&body, false), OfficeFormat::Docx);
    let first = render(&mut session, 0);
    assert!(first.contains("Empty content window"));
    assert!(!first.contains("unsupported layout contents are omitted"));
    assert!(!first.contains("Next-visible"));
    assert!(first.contains("/office/v1/document/0/block/16.html"));
    let next = render(&mut session, 16);
    assert!(
        next.contains("Next-visible") && next.contains("unsupported layout contents are omitted")
    );
}

#[test]
fn physically_long_unselected_table_prefix_does_not_materialize_cells() {
    const PREFIX: usize = 4096;
    let rows = "<g:table-row><g:table-cell g:number-columns-repeated=\"16384\"/></g:table-row>"
        .repeat(PREFIX);
    let table = format!(
        "<g:table g:name=\"Physical rows\">{rows}<g:table-row><g:table-cell><t:p>Selected final cell</t:p></g:table-cell></g:table-row></g:table>"
    );
    let docx_rows =
        "<different:tr><different:tc><different:p/></different:tc></different:tr>".repeat(PREFIX);
    let word = format!(
        "<different:tbl>{docx_rows}<different:tr><different:tc><different:p><different:r><different:t>Selected final cell</different:t></different:r></different:p></different:tc></different:tr></different:tbl>"
    );
    for (source, format, target) in [
        (
            odf(&table, "text"),
            OfficeFormat::Odt,
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: PREFIX as u32,
            },
        ),
        (
            odf(
                &format!("<d:page><d:frame>{table}</d:frame></d:page>"),
                "presentation",
            ),
            OfficeFormat::Odp,
            OfficeTarget::SlideWindow {
                slide: SlideId(0),
                block: PREFIX as u32,
            },
        ),
        (
            docx(&word, false),
            OfficeFormat::Docx,
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: PREFIX as u32,
            },
        ),
    ] {
        let mut session = open(source, format);
        crate::document::take_materialized_cells();
        let page = session.render(target, &Cancellation::default()).unwrap();
        assert_eq!(
            crate::document::take_materialized_cells(),
            1,
            "only the selected cell may materialize"
        );
        assert!(
            String::from_utf8(page.bytes.to_vec())
                .unwrap()
                .contains("Selected final cell")
        );
    }
}

#[test]
fn skipped_physical_rows_still_validate_repeat_coordinates() {
    let table = r#"<g:table g:name="Invalid prefix"><g:table-row><g:table-cell g:number-columns-repeated="16384"/><g:table-cell/></g:table-row><g:table-row><g:table-cell><t:p>Selected</t:p></g:table-cell></g:table-row></g:table>"#;
    for (source, format, target) in [
        (
            odf(table, "text"),
            OfficeFormat::Odt,
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: 1,
            },
        ),
        (
            odf(
                &format!("<d:page><d:frame>{table}</d:frame></d:page>"),
                "presentation",
            ),
            OfficeFormat::Odp,
            OfficeTarget::SlideWindow {
                slide: SlideId(0),
                block: 1,
            },
        ),
    ] {
        let mut session = open(source, format);
        assert!(matches!(
            session.render(target, &Cancellation::default()),
            Err(OfficeError::LimitExceeded(_))
        ));
    }
}
