//! Real-package ODS boundaries. This module is unregistered until the shared format gate opens.
use super::*;
use crate::{Cancellation, OfficeSource};
use std::{
    io::{self, Cursor, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

struct Source {
    bytes: Vec<u8>,
    calls: AtomicUsize,
    cancel_at: AtomicUsize,
}
impl OfficeSource for Source {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8], cancel: &Cancellation) -> io::Result<usize> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if cancel.is_cancelled() {
            return Err(io::Error::other(OfficeError::Cancelled));
        }
        let remaining = self
            .bytes
            .get(offset as usize..)
            .ok_or_else(|| io::Error::other("fixture offset"))?;
        let count = remaining.len().min(output.len()).min(37);
        output[..count].copy_from_slice(&remaining[..count]);
        if self.cancel_at.load(Ordering::Relaxed) == call {
            cancel.cancel();
        }
        Ok(count)
    }
}

fn source(body: &str, equivalent_prefixes: bool) -> Arc<Source> {
    let mut content = format!(
        r#"<o:document-content xmlns:o="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:t="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:g="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:d="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" o:version="1.3"><o:body><o:spreadsheet>{body}</o:spreadsheet></o:body></o:document-content>"#
    );
    if equivalent_prefixes {
        for (prefix, replacement) in [
            ("o", "office2"),
            ("t", "words"),
            ("g", "grid"),
            ("d", "drawing2"),
        ] {
            content = content
                .replace(&format!("<{prefix}:"), &format!("<{replacement}:"))
                .replace(&format!("</{prefix}:"), &format!("</{replacement}:"))
                .replace(&format!(" {prefix}:"), &format!(" {replacement}:"))
                .replace(
                    &format!("xmlns:{prefix}="),
                    &format!("xmlns:{replacement}="),
                );
        }
    }
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes, compression) in [
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet".to_owned(), CompressionMethod::Stored),
        ("META-INF/manifest.xml", r#"<m:manifest xmlns:m="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" m:version="1.3"><m:file-entry m:full-path="/" m:media-type="application/vnd.oasis.opendocument.spreadsheet"/><m:file-entry m:full-path="content.xml" m:media-type="text/xml"/></m:manifest>"#.to_owned(), CompressionMethod::Deflated),
        ("content.xml", content, CompressionMethod::Deflated),
    ] {
        writer.start_file(name, SimpleFileOptions::default().compression_method(compression)).unwrap();
        writer.write_all(bytes.as_bytes()).unwrap();
    }
    Arc::new(Source {
        bytes: writer.finish().unwrap().into_inner(),
        calls: AtomicUsize::new(0),
        cancel_at: AtomicUsize::new(usize::MAX),
    })
}
fn open(body: &str) -> (Package, Ods) {
    let mut package = Package::open(source(body, false), &Cancellation::default()).unwrap();
    let ods = Ods::load(&mut package).unwrap();
    (package, ods)
}
fn render(package: &mut Package, ods: &Ods, target: OfficeTarget) -> (Window, String) {
    package.begin(&Cancellation::default()).unwrap();
    let window = window(package, target).unwrap();
    let sheet = match target {
        OfficeTarget::SheetFirst { sheet } | OfficeTarget::SheetWindow { sheet, .. } => sheet,
        _ => panic!("sheet target expected"),
    };
    let html =
        String::from_utf8(crate::html::sheet(&ods.catalog, sheet, &window).unwrap()).unwrap();
    (window, html)
}

#[test]
fn ods_sparse_repeats_skip_empty_rows_and_preserve_named_sheet_membership() {
    let body = r#"<g:table g:name="Sparse &amp; safe"><g:table-row g:number-rows-repeated="999999"><g:table-cell g:number-columns-repeated="16384"/></g:table-row><g:table-row><g:table-cell g:number-columns-repeated="16383"/><g:table-cell o:value-type="string"><t:p>Far &lt;cell&gt;</t:p></g:table-cell></g:table-row></g:table><g:table g:name="Second"><g:table-row><g:table-cell o:value-type="float" o:value="42"/></g:table-row></g:table>"#;
    for prefixes in [false, true] {
        let mut package = Package::open(source(body, prefixes), &Cancellation::default()).unwrap();
        let ods = Ods::load(&mut package).unwrap();
        assert_eq!(ods.catalog.sheets.len(), 2);
        assert_eq!(ods.catalog.sheets[0].name, "Sparse & safe");
        let (window, html) = render(
            &mut package,
            &ods,
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
        );
        assert_eq!(
            (window.row, window.column, window.cells.len()),
            (1_000_000, 16_384, 1)
        );
        assert!(
            html.contains("Far &lt;cell&gt;") && html.contains("1000000") && html.contains("XFD")
        );
        assert!(html.len() < 100_000);
        assert!(
            render(
                &mut package,
                &ods,
                OfficeTarget::SheetFirst { sheet: SheetId(1) }
            )
            .1
            .contains("42")
        );
        assert!(matches!(
            window_target(&mut package, 2),
            Err(OfficeError::InvalidTarget)
        ));
    }
}
fn window_target(package: &mut Package, sheet: u16) -> Result<Window, OfficeError> {
    package.begin(&Cancellation::default())?;
    window(
        package,
        OfficeTarget::SheetFirst {
            sheet: SheetId(sheet),
        },
    )
}

#[test]
fn ods_repeated_populated_rows_materialize_only_window_intersection() {
    let body = r#"<g:table g:name="Repeated"><g:table-row g:number-rows-repeated="1000000"><g:table-cell g:number-columns-repeated="16384" o:value-type="string"><t:p>Repeated-value</t:p></g:table-cell></g:table-row></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, html) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetWindow {
            sheet: SheetId(0),
            row: 999_900,
            column: 16_380,
        },
    );
    assert_eq!(window.cells.len(), 505);
    assert_eq!(
        (window.cells[0].row, window.cells[0].column),
        (999_900, 16_380)
    );
    assert_eq!(
        (window.cells[504].row, window.cells[504].column),
        (1_000_000, 16_384)
    );
    assert_eq!(html.matches("Repeated-value").count(), 505);
    assert!(html.len() < 100_000);
}

#[test]
fn ods_repeat_overflow_and_unsupported_only_drawing_are_explicit() {
    for body in [
        r#"<g:table g:name="Bad"><g:table-row g:number-rows-repeated="4294967296"/></g:table>"#,
        r#"<g:table g:name="Bad"><g:table-row g:number-rows-repeated="0"/></g:table>"#,
        r#"<g:table g:name="Bad"><g:table-row g:number-rows-repeated="1048577"/></g:table>"#,
        r#"<g:table g:name="Bad"><g:table-row><g:table-cell g:number-columns-repeated="16385"/></g:table-row></g:table>"#,
        r#"<g:table g:name="Bad"><g:table-row><g:table-cell g:number-columns-repeated="0"/></g:table-row></g:table>"#,
        r#"<g:table g:name="Drawing"><d:frame><d:image/></d:frame></g:table>"#,
    ] {
        let (mut package, _) = open(body);
        assert!(window_target(&mut package, 0).is_err());
    }
}

#[test]
fn ods_cached_formula_values_and_inert_fallback_preserve_stored_text() {
    let body = r#"<g:table g:name="Values"><g:table-row><g:table-cell o:value-type="float" o:value="42" g:formula="of:=SUM([.A2])"><t:p/></g:table-cell><g:table-cell g:formula="of:=HYPERLINK(&quot;https://invalid.example/&quot;;&quot;&lt;unsafe&gt;&quot;)"/><g:table-cell o:value-type="boolean" o:boolean-value="true"/><g:table-cell o:value-type="date" o:date-value="2026-11-17"/><g:table-cell o:value-type="time" o:time-value="PT1H2M"/><g:table-cell o:value-type="string" o:string-value="&lt;script&gt;&amp;"/></g:table-row></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, html) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
    );
    assert_eq!(window.cells[0].text, "42");
    assert!(
        window.cells[1]
            .text
            .starts_with("Formula (not evaluated): of:=HYPERLINK(")
    );
    assert_eq!(window.cells[2].text, "TRUE");
    assert_eq!(window.cells[3].text, "2026-11-17");
    assert_eq!(window.cells[4].text, "PT1H2M");
    assert!(html.contains("&lt;script&gt;&amp;") && !html.contains("<script>"));
    assert!(!html.contains("href=\"https:") && !html.contains("href=\"file:"));
    assert!(!html.contains("Formula (not evaluated): of:=SUM"));
}

#[test]
fn ods_zero_and_large_space_repetitions_are_bounded_and_visible() {
    let body = r#"<g:table g:name="Spaces"><g:table-row><g:table-cell><t:p>A<t:s t:c="0"/>B<t:s t:c="3"/>C</t:p></g:table-cell><g:table-cell><t:p>X<t:s t:c="4294967295"/></t:p></g:table-cell></g:table-row></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, _) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
    );
    assert_eq!(window.cells[0].text, "AB   C");
    assert!(window.cells[1].text.len() <= 32 * 1024);
    assert!(window.cells[1].text.ends_with("[truncated]"));
}

#[test]
fn ods_unselected_repeated_spaces_do_not_hide_a_far_value_or_expand_prefix() {
    let body = r#"<g:table g:name="Far"><g:table-row><g:table-cell g:number-columns-repeated="16383"><t:p><t:s t:c="4294967295"/></t:p></g:table-cell><g:table-cell><t:p>Selected far value</t:p></g:table-cell></g:table-row></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, _) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetWindow {
            sheet: SheetId(0),
            row: 1,
            column: 16_384,
        },
    );
    assert_eq!(window.cells.len(), 1);
    assert_eq!(window.cells[0].text, "Selected far value");
}

#[test]
fn ods_next_populated_region_includes_the_unrendered_repeat_rectangle() {
    let body = r#"<g:table g:name="Dense repeats"><g:table-row g:number-rows-repeated="1000000"><g:table-cell g:number-columns-repeated="16384"><t:p>x</t:p></g:table-cell></g:table-row></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, html) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
    );
    assert_eq!(window.cells.len(), 4096);
    assert_eq!(window.next, Some((1, 33)));
    assert!(html.contains("/office/v1/sheet/0/row/1/col/33.html"));
}

#[test]
fn ods_empty_range_differs_from_unsupported_drawing_and_unknown_namespace() {
    let (mut package, ods) = open(
        r#"<g:table g:name="Empty"><g:table-row g:number-rows-repeated="1048576"><g:table-cell g:number-columns-repeated="16384"/></g:table-row></g:table>"#,
    );
    let (window, html) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
    );
    assert!(window.cells.is_empty() && html.contains("Empty cell range"));
    let (mut package, _) = open(
        r#"<g:table g:name="Other"><g:table-row><g:table-cell><q:p xmlns:q="urn:not-odf">Pretend cell</q:p></g:table-cell></g:table-row></g:table>"#,
    );
    assert!(matches!(
        window_target(&mut package, 0),
        Err(OfficeError::Unsupported(_))
    ));
}

#[test]
fn ods_selected_window_text_budget_fails_before_repeat_materialization_grows() {
    let text = "x".repeat(2000);
    let body = format!(
        r#"<g:table g:name="Budget"><g:table-row g:number-rows-repeated="1000000"><g:table-cell g:number-columns-repeated="16384"><t:p>{text}</t:p></g:table-cell></g:table-row></g:table>"#
    );
    let (mut package, _) = open(&body);
    assert!(matches!(
        window_target(&mut package, 0),
        Err(OfficeError::LimitExceeded("ODS window text"))
    ));
}

#[test]
fn ods_cancelled_source_replay_returns_no_partial_window() {
    let body = format!(
        r#"<g:table g:name="Cancelled"><g:table-row><g:table-cell><t:p>{}</t:p></g:table-cell></g:table-row></g:table>"#,
        "data".repeat(4000)
    );
    let captured = source(&body, false);
    let mut package = Package::open(captured.clone(), &Cancellation::default()).unwrap();
    Ods::load(&mut package).unwrap();
    let cancel = Cancellation::default();
    package.begin(&cancel).unwrap();
    captured.cancel_at.store(
        captured.calls.load(Ordering::Relaxed) + 3,
        Ordering::Relaxed,
    );
    assert!(matches!(
        window(&mut package, OfficeTarget::SheetFirst { sheet: SheetId(0) }),
        Err(OfficeError::Cancelled)
    ));
    assert!(cancel.is_cancelled());
}

#[test]
fn ods_grouped_rows_keep_paragraph_order_and_external_link_text_inert() {
    let body = r#"<g:table g:name="Groups"><g:table-column g:number-columns-repeated="2"/><g:table-source xmlns:x="http://www.w3.org/1999/xlink" x:href="file:///never-open"/><g:table-header-rows><g:table-row><g:table-cell><t:p>Header</t:p></g:table-cell></g:table-row></g:table-header-rows><g:table-row-group><g:table-rows><g:table-row><g:table-cell><t:p>A <t:span>B</t:span></t:p><t:p>C<t:a xmlns:x="http://www.w3.org/1999/xlink" x:href="https://invalid.example/">D</t:a><t:tab/><t:line-break/></t:p></g:table-cell></g:table-row></g:table-rows></g:table-row-group></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, html) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
    );
    assert_eq!(window.cells[0].row, 1);
    assert_eq!(window.cells[0].text, "Header");
    assert_eq!(window.cells[1].row, 2);
    assert_eq!(window.cells[1].text, "A B\nCD\t\n");
    assert!(!html.contains("file:///") && !html.contains("https://invalid.example/"));
}

#[test]
fn ods_wrong_body_structure_and_lookalike_namespaces_are_rejected() {
    for body in [
        r#"<g:table g:name="First"/></o:spreadsheet></o:body><o:body><o:spreadsheet><g:table g:name="Second"/>"#,
        r#"<q:table xmlns:q="urn:not-odf" q:name="Pretend"/>"#,
    ] {
        let mut package = Package::open(source(body, false), &Cancellation::default()).unwrap();
        assert!(Ods::load(&mut package).is_err());
    }
}

#[test]
fn ods_empty_cached_strings_do_not_anchor_the_preview_before_visible_values() {
    let body = r#"<g:table g:name="Empty cache"><g:table-row g:number-rows-repeated="999999"><g:table-cell g:number-columns-repeated="16384" o:value-type="string" o:string-value=""/></g:table-row><g:table-row><g:table-cell o:value-type="float" o:value="7"/></g:table-row></g:table>"#;
    let (mut package, ods) = open(body);
    let (window, _) = render(
        &mut package,
        &ods,
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
    );
    assert_eq!(
        (window.row, window.column, window.cells.len()),
        (1_000_000, 1, 1)
    );
    assert_eq!(window.cells[0].text, "7");
}
