//! Generated memory fixtures only. Execution requires the coordinated build lease.
use midnight_office_preview::{
    Cancellation, OfficeError, OfficeFormat, OfficeSession, OfficeSource, OfficeTarget, SheetId,
};
use std::{
    io::{self, Cursor, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zip::{ZipWriter, write::SimpleFileOptions};

struct Source {
    bytes: Vec<u8>,
    max_read: AtomicUsize,
    calls: AtomicUsize,
    delivered: AtomicUsize,
}
impl OfficeSource for Source {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8], _: &Cancellation) -> io::Result<usize> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.max_read.fetch_max(output.len(), Ordering::Relaxed);
        let offset = usize::try_from(offset).map_err(io::Error::other)?;
        let bytes = self
            .bytes
            .get(offset..)
            .ok_or_else(|| io::Error::other("offset"))?;
        let size = output.len().min(bytes.len());
        self.delivered.fetch_add(size, Ordering::Relaxed);
        output[..size].copy_from_slice(&bytes[..size]);
        Ok(size)
    }
}
fn source(bytes: Vec<u8>) -> Arc<Source> {
    Arc::new(Source {
        bytes,
        max_read: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        delivered: AtomicUsize::new(0),
    })
}
fn package(sheet: &str, shared: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<w:workbook xmlns:w="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:sheets><w:sheet name="První &amp; &lt;sheet&gt;" sheetId="1" r:id="a"/><w:sheet name="Second" sheetId="2" r:id="b"/></w:sheets></w:workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="a" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="b" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="s" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="external" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://invalid.example/" TargetMode="External"/></Relationships>"#,
        ),
        ("xl/worksheets/sheet1.xml", sheet),
        (
            "xl/worksheets/sheet2.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Second sheet</t></is></c></row></sheetData></worksheet>"#,
        ),
        ("xl/sharedStrings.xml", shared),
    ];
    for (name, text) in parts {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
const SHEET: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:XFD1048576"/><sheetData><row r="1"><c r="A1" t="s"><v>1</v></c><c r="B1"><f>1+2</f><v>3</v></c><c r="C1"><f>HYPERLINK(&quot;https://invalid.example&quot;)</f></c></row><row r="1000000"><c r="XFD1000000" t="inlineStr"><is><t>Far cell</t></is></c></row></sheetData></worksheet>"#;
const SHARED: &str = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Unused</t></si><si><r><t>Žluťoučký &lt;script&gt;</t></r><r><t> &amp; safe</t></r></si></sst>"#;
fn open(bytes: Vec<u8>) -> OfficeSession {
    OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()).unwrap()
}
fn html(session: &mut OfficeSession, target: OfficeTarget) -> String {
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
fn catalog_and_windows_render_real_cells_and_numeric_navigation() {
    let captured = source(package(SHEET, SHARED));
    let mut session = OfficeSession::open(
        captured.clone(),
        OfficeFormat::Xlsx,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(session.catalog().sheets.len(), 2);
    let catalog = html(&mut session, OfficeTarget::Catalog);
    assert!(catalog.contains("První &amp; &lt;sheet&gt;"));
    assert!(catalog.contains("/office/v1/sheet/1/first.html"));
    let page = html(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(0) });
    assert!(page.contains("Žluťoučký &lt;script&gt; &amp; safe"));
    assert!(page.contains(">3</td>"));
    assert!(page.contains("Formula (not evaluated):"));
    assert!(!page.contains("href=\"https:"));
    assert!(page.contains("/office/v1/sheet/0/row/129/col/1.html"));
    assert!(page.contains("/office/v1/sheet/0/row/1/col/33.html"));
    assert!(page.contains("/office/v1/sheet/0/row/1000000/col/16384.html"));
    let far = html(
        &mut session,
        OfficeTarget::SheetWindow {
            sheet: SheetId(0),
            row: 1_000_000,
            column: 16_384,
        },
    );
    assert!(far.contains("Far cell"));
    assert!(far.contains("XFD"));
    let second = html(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(1) });
    assert!(second.contains("Second sheet"));
    assert!(captured.max_read.load(Ordering::Relaxed) <= 65_536);
}

#[test]
fn source_reads_stop_on_pre_cancel_and_wrong_package_families_are_rejected() {
    let captured = source(package(SHEET, SHARED));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        OfficeSession::open(captured.clone(), OfficeFormat::Xlsx, &cancel),
        Err(OfficeError::Cancelled)
    ));
    assert_eq!(captured.calls.load(Ordering::Relaxed), 0);
    for format in [
        OfficeFormat::Docx,
        OfficeFormat::Pptx,
        OfficeFormat::Odt,
        OfficeFormat::Ods,
        OfficeFormat::Odp,
    ] {
        let result = OfficeSession::open(captured.clone(), format, &Cancellation::default());
        match format {
            OfficeFormat::Docx | OfficeFormat::Pptx => {
                assert!(matches!(result, Err(OfficeError::Unsupported(_))))
            }
            OfficeFormat::Odt | OfficeFormat::Ods | OfficeFormat::Odp => {
                assert!(matches!(result, Err(OfficeError::Malformed(_))))
            }
            OfficeFormat::Xlsx => unreachable!(),
        }
    }
}

#[test]
fn exact_targets_reject_url_smuggling_and_foreign_sheet_ids() {
    let session = open(package(SHEET, SHARED));
    for path in [
        "/office/v1/sheet/00/first.html",
        "/office/v1/sheet/2/first.html",
        "/office/v1/catalog.html?x",
        "/office/v1/catalog.html#x",
        "/office/v1/sheet/0/row/0/col/1.html",
        "/office/v1/sheet/0/row/1048577/col/1.html",
        "/office/v1/sheet/0/row/1/col/16385.html",
        "/office/v1/%63atalog.html",
        "https://invalid.example/office/v1/catalog.html",
    ] {
        assert!(session.parse_target(path).is_err(), "{path}");
    }
}

#[test]
fn xml_bombs_entities_and_utf16_are_rejected() {
    let huge = format!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row><c r=\"A1\" t=\"inlineStr\"><is><t>{}</t></is></c></row></sheetData></worksheet>",
        "x".repeat(65_537)
    );
    let depth = format!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">{}{}</worksheet>",
        "<x>".repeat(129),
        "</x>".repeat(129)
    );
    for sheet in [
        huge.as_str(),
        depth.as_str(),
        "<!DOCTYPE a [<!ENTITY x SYSTEM 'file:///secret'>]><a>&x;</a>",
        "<?xml version='1.0' encoding='UTF-16'?><worksheet/>",
        "<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData><row><c r='A1' t='inlineStr'><is><t>&unknown;</t></is></c></row></sheetData></worksheet>",
    ] {
        let mut session = open(package(sheet, SHARED));
        assert!(
            session
                .render(
                    OfficeTarget::SheetFirst { sheet: SheetId(0) },
                    &Cancellation::default()
                )
                .is_err()
        );
    }
}

#[test]
fn malformed_zip_and_forged_directory_counts_fail_before_indexing() {
    assert!(
        OfficeSession::open(
            source(b"not zip".to_vec()),
            OfficeFormat::Xlsx,
            &Cancellation::default()
        )
        .is_err()
    );
    let mut bytes = package(SHEET, SHARED);
    let end = bytes.len() - 22;
    bytes[end + 8..end + 12].fill(0);
    assert!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()).is_err()
    );
}

#[test]
fn empty_sheet_and_unsupported_drawing_are_distinguishable() {
    let mut empty = open(package(
        "<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData/></worksheet>",
        SHARED,
    ));
    assert!(html(&mut empty, OfficeTarget::SheetFirst { sheet: SheetId(0) }).contains("Empty"));
    let mut drawing = open(package(
        "<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData/><drawing/></worksheet>",
        SHARED,
    ));
    assert!(matches!(
        drawing.render(
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
            &Cancellation::default()
        ),
        Err(OfficeError::Unsupported(_))
    ));
}

fn rewrite(bytes: Vec<u8>, edit: impl Fn(&str, String) -> String, stored: bool) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).unwrap();
        let name = member.name().to_owned();
        let mut text = String::new();
        member.read_to_string(&mut text).unwrap();
        let method = if stored {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        writer
            .start_file(
                &name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(edit(&name, text).as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn package_root_relationships_and_strict_equivalent_prefixes_are_supported() {
    let bytes = rewrite(
        package(SHEET, SHARED),
        |name, text| {
            let text = text
                .replace(
                    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                    "http://purl.oclc.org/ooxml/spreadsheetml/main",
                )
                .replace(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                    "http://purl.oclc.org/ooxml/officeDocument/relationships",
                );
            if name.ends_with(".rels") {
                text.replace("Target=\"xl/", "Target=\"/xl/")
                    .replace("Target=\"worksheets/", "Target=\"/xl/worksheets/")
                    .replace(
                        "Target=\"sharedStrings.xml\"",
                        "Target=\"/xl/sharedStrings.xml\"",
                    )
            } else {
                text
            }
        },
        false,
    );
    let mut session = open(bytes);
    assert!(
        html(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(0) }).contains("Žluťoučký")
    );
}

#[test]
fn escaping_duplicate_and_uri_relationships_are_rejected() {
    for target in [
        "../../escape.xml",
        "//authority/member",
        "C:/file.xml",
        "file:///secret",
        "%2e%2e/escape",
        "..\\escape.xml",
    ] {
        let bytes = rewrite(
            package(SHEET, SHARED),
            |name, text| {
                if name == "xl/_rels/workbook.xml.rels" {
                    text.replace("worksheets/sheet1.xml", target)
                } else {
                    text
                }
            },
            false,
        );
        assert!(
            matches!(
                OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
                Err(OfficeError::Malformed(_))
            ),
            "{target}"
        );
    }
    let bytes = rewrite(
        package(SHEET, SHARED),
        |name, text| {
            if name == "xl/_rels/workbook.xml.rels" {
                text.replace("Id=\"b\"", "Id=\"a\"")
            } else {
                text
            }
        },
        false,
    );
    assert!(matches!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
        Err(OfficeError::Malformed(_))
    ));
}

#[test]
fn indexed_member_drop_does_not_read_unselected_shared_string_tail() {
    let suffix = "<si><t>".to_owned() + &"x".repeat(4 * 1024 * 1024) + "</t></si></sst>";
    let bytes = rewrite(
        package(SHEET, SHARED),
        |name, text| {
            if name == "xl/sharedStrings.xml" {
                text.replace("</sst>", &suffix)
            } else {
                text
            }
        },
        true,
    );
    let captured = source(bytes);
    let mut session = OfficeSession::open(
        captured.clone(),
        OfficeFormat::Xlsx,
        &Cancellation::default(),
    )
    .unwrap();
    let before = captured.delivered.load(Ordering::Relaxed);
    assert!(
        html(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(0) }).contains("Žluťoučký")
    );
    assert!(captured.delivered.load(Ordering::Relaxed) - before < 128 * 1024);
}

#[test]
fn short_source_reads_are_completed_and_cancelled_interrupted_read_does_not_retry_forever() {
    struct Short(Arc<Source>);
    impl OfficeSource for Short {
        fn len(&self) -> u64 {
            self.0.len()
        }
        fn read_at(
            &self,
            offset: u64,
            output: &mut [u8],
            cancel: &Cancellation,
        ) -> io::Result<usize> {
            let length = output.len().min(3);
            self.0.read_at(offset, &mut output[..length], cancel)
        }
    }
    let mut session = OfficeSession::open(
        Arc::new(Short(source(package(SHEET, SHARED)))),
        OfficeFormat::Xlsx,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(
        html(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(0) }).contains("Žluťoučký")
    );
    struct CancelRead {
        calls: AtomicUsize,
    }
    impl OfficeSource for CancelRead {
        fn len(&self) -> u64 {
            100
        }
        fn read_at(&self, _: u64, _: &mut [u8], cancel: &Cancellation) -> io::Result<usize> {
            assert_eq!(
                self.calls.fetch_add(1, Ordering::Relaxed),
                0,
                "cancelled input was retried"
            );
            cancel.cancel();
            Err(io::Error::from(io::ErrorKind::Interrupted))
        }
    }
    assert!(matches!(
        OfficeSession::open(
            Arc::new(CancelRead {
                calls: AtomicUsize::new(0)
            }),
            OfficeFormat::Xlsx,
            &Cancellation::default()
        ),
        Err(OfficeError::Cancelled)
    ));
}

#[test]
fn zip_extras_comments_encryption_and_links_have_explicit_states() {
    let original = package(SHEET, SHARED);
    let central = original
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    for field in [30usize, 32] {
        let mut bytes = original.clone();
        bytes[central + field] = 1;
        assert!(matches!(
            OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
            Err(OfficeError::Unsupported(_))
        ));
    }
    let mut bytes = original.clone();
    bytes[central + 8] |= 1;
    assert!(matches!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
        Err(OfficeError::Encrypted)
    ));
    let mut bytes = original;
    bytes[central + 40..central + 42].copy_from_slice(&0xa000u16.to_le_bytes());
    assert!(matches!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
        Err(OfficeError::Malformed(_))
    ));
}

#[test]
fn long_cells_are_labelled_and_window_text_limit_is_enforced() {
    let cell = format!(
        "<c r=\"A1\" t=\"inlineStr\"><is><t>{}</t></is></c>",
        "ž".repeat(20_000)
    );
    let sheet = format!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\">{cell}</row></sheetData></worksheet>"
    );
    let mut session = open(package(&sheet, SHARED));
    let page = html(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(0) });
    assert!(page.contains("[truncated]"));
    assert!(page.len() < 50_000);
    let mut cells = String::new();
    for column in b'A'..=b'Q' {
        cells.push_str(&format!(
            "<c r=\"{}1\" t=\"inlineStr\"><is><t>{}</t></is></c>",
            char::from(column),
            "x".repeat(32_000)
        ));
    }
    let sheet = format!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\">{cells}</row></sheetData></worksheet>"
    );
    let mut session = open(package(&sheet, SHARED));
    assert!(matches!(
        session.render(
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
            &Cancellation::default()
        ),
        Err(OfficeError::LimitExceeded(_))
    ));
}

#[test]
fn session_can_move_to_backend_lane() {
    fn assert_send<T: Send>() {}
    assert_send::<OfficeSession>();
}

#[test]
fn advertised_container_count_and_metadata_limits_precede_index_allocation() {
    struct Oversized;
    impl OfficeSource for Oversized {
        fn len(&self) -> u64 {
            256 * 1024 * 1024 + 1
        }
        fn read_at(&self, _: u64, _: &mut [u8], _: &Cancellation) -> io::Result<usize> {
            panic!("oversized container read before admission");
        }
    }
    assert!(matches!(
        OfficeSession::open(
            Arc::new(Oversized),
            OfficeFormat::Xlsx,
            &Cancellation::default()
        ),
        Err(OfficeError::LimitExceeded(_))
    ));
    let original = package(SHEET, SHARED);
    let end = original.len() - 22;
    let mut bytes = original.clone();
    bytes[end + 8..end + 10].copy_from_slice(&10_001u16.to_le_bytes());
    bytes[end + 10..end + 12].copy_from_slice(&10_001u16.to_le_bytes());
    assert!(matches!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
        Err(OfficeError::LimitExceeded(_))
    ));
    let mut bytes = original;
    bytes[end + 12..end + 16].copy_from_slice(&(8 * 1024 * 1024 + 1u32).to_le_bytes());
    assert!(matches!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
        Err(OfficeError::LimitExceeded(_))
    ));
}

#[test]
fn duplicate_package_names_and_cell_coordinates_are_rejected() {
    let mut bytes = package(SHEET, SHARED);
    let from = b"xl/worksheets/sheet2.xml";
    let positions: Vec<_> = bytes
        .windows(from.len())
        .enumerate()
        .filter_map(|(i, name)| (name == from).then_some(i))
        .collect();
    for position in positions {
        bytes[position + 19] = b'1';
    }
    assert!(matches!(
        OfficeSession::open(source(bytes), OfficeFormat::Xlsx, &Cancellation::default()),
        Err(OfficeError::Malformed(_))
    ));
    let sheet = "<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData><row r='1'><c r='A1'><v>1</v></c><c r='A1'><v>2</v></c></row></sheetData></worksheet>";
    let mut session = open(package(sheet, SHARED));
    assert!(matches!(
        session.render(
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
            &Cancellation::default()
        ),
        Err(OfficeError::Malformed(_))
    ));
}

#[test]
fn missing_large_shared_string_id_fails_without_dimension_allocation() {
    let sheet = "<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData><row r='1'><c r='A1' t='s'><v>4294967295</v></c></row></sheetData></worksheet>";
    let mut session = open(package(sheet, SHARED));
    assert!(matches!(
        session.render(
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
            &Cancellation::default()
        ),
        Err(OfficeError::Malformed(_))
    ));
}

#[test]
fn archive_comments_with_shadow_eocd_and_alternate_directory_are_rejected_pre_index() {
    let original = package(SHEET, SHARED);
    for alternate_directory in [false, true] {
        let mut bytes = original.clone();
        let outer = bytes.len() - 22;
        let mut comment = Vec::new();
        if alternate_directory {
            // Plausible alternate central record in the outer archive comment.
            comment.extend_from_slice(b"PK\x01\x02");
            comment.extend_from_slice(&[0; 42]);
        }
        let mut shadow = [0u8; 22];
        shadow[..4].copy_from_slice(b"PK\x05\x06");
        comment.extend_from_slice(&shadow);
        comment.extend_from_slice(b"non-EOF trailer");
        bytes[outer + 20..outer + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&comment);
        let captured = source(bytes);
        assert!(matches!(
            OfficeSession::open(
                captured.clone(),
                OfficeFormat::Xlsx,
                &Cancellation::default()
            ),
            Err(OfficeError::Unsupported(_))
        ));
        assert_eq!(
            captured.calls.load(Ordering::Relaxed),
            2,
            "shadow input reached central indexing"
        );
    }
}

#[test]
fn sparse_shadow_zip64_extensible_claim_is_rejected_without_large_read_or_allocation() {
    // Synthetic 192 MiB source: no large fixture is allocated. The valid outer
    // empty directory and its comment are in a tiny tail; the ZIP64 record at
    // offset zero advertises a large extensible sector ending at the locator.
    struct Sparse {
        length: u64,
        tail_offset: u64,
        tail: Vec<u8>,
        prefix: [u8; 56],
        calls: AtomicUsize,
    }
    impl OfficeSource for Sparse {
        fn len(&self) -> u64 {
            self.length
        }
        fn read_at(&self, offset: u64, output: &mut [u8], _: &Cancellation) -> io::Result<usize> {
            assert!(
                self.calls.fetch_add(1, Ordering::Relaxed) < 3,
                "shadow ZIP64 reached eager indexing"
            );
            let length = output
                .len()
                .min(self.length.saturating_sub(offset) as usize);
            for (index, byte) in output[..length].iter_mut().enumerate() {
                let at = offset + index as u64;
                *byte = if at < self.prefix.len() as u64 {
                    self.prefix[at as usize]
                } else if at >= self.tail_offset {
                    self.tail[(at - self.tail_offset) as usize]
                } else {
                    0
                };
            }
            Ok(length)
        }
    }
    let outer_offset = 192 * 1024 * 1024u64;
    let mut tail = vec![0u8; 22 + 20 + 22 + 4];
    tail[..4].copy_from_slice(b"PK\x05\x06");
    tail[16..20].copy_from_slice(&(outer_offset as u32).to_le_bytes());
    tail[20..22].copy_from_slice(&46u16.to_le_bytes());
    tail[22..26].copy_from_slice(b"PK\x06\x07");
    tail[38..42].copy_from_slice(&1u32.to_le_bytes());
    tail[42..46].copy_from_slice(b"PK\x05\x06");
    tail[50..54].fill(0xff);
    let mut prefix = [0u8; 56];
    prefix[..4].copy_from_slice(b"PK\x06\x06");
    prefix[4..12].copy_from_slice(&(outer_offset + 22 - 12).to_le_bytes());
    let captured = Arc::new(Sparse {
        length: outer_offset + tail.len() as u64,
        tail_offset: outer_offset,
        tail,
        prefix,
        calls: AtomicUsize::new(0),
    });
    assert!(matches!(
        OfficeSession::open(
            captured.clone(),
            OfficeFormat::Xlsx,
            &Cancellation::default()
        ),
        Err(OfficeError::Unsupported(_))
    ));
    assert_eq!(captured.calls.load(Ordering::Relaxed), 3);
}
