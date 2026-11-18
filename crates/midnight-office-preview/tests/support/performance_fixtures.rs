//! Real ZIP method-8 fixtures; all compression completes before a measured baseline.
use crate::source::{Source, Tape, TapeWriter};
use std::{
    io::{self, Write},
    ops::Range,
    sync::Arc,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const SHEET_START: &[u8] =
    b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>";
pub const SHEET_END: &[u8] = b"</sheetData></worksheet>";
pub const FIRST_TEXT: &str = "Observed first &amp; safe";
pub const FAR_TEXT: &str = "Observed far cell";

const PARTS: [(&str, &str); 6] = [
    (
        "[Content_Types].xml",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#,
    ),
    (
        "_rels/.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="root" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    ),
    (
        "xl/workbook.xml",
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Measured" sheetId="1" r:id="first"/><sheet name="Second" sheetId="2" r:id="second"/></sheets></workbook>"#,
    ),
    (
        "xl/_rels/workbook.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="first" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="second" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="strings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#,
    ),
    (
        "xl/worksheets/sheet2.xml",
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Observed second</t></is></c></row></sheetData></worksheet>"#,
    ),
    (
        "xl/sharedStrings.xml",
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>Observed first &amp; safe</t></si><si><t>Observed far cell</t></si></sst>"#,
    ),
];
const SHEET_NAME: &str = "xl/worksheets/sheet1.xml";
const SHARED_STRINGS_NAME: &str = "xl/sharedStrings.xml";

pub struct Fixture {
    pub tape: Arc<Tape>,
    pub worksheet: Range<u64>,
    pub shared_strings: Range<u64>,
    pub worksheet_uncompressed: u64,
    pub entries: usize,
    pub cumulative_name_bytes: usize,
}
impl Fixture {
    pub fn source(&self, chunk: usize) -> Arc<Source> {
        Source::new(
            self.tape.clone(),
            self.worksheet.clone(),
            self.shared_strings.clone(),
            chunk,
        )
    }
}

fn options(level: i64) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(level))
}

fn comment() -> Vec<u8> {
    let mut comment = Vec::with_capacity(32 * 1024);
    comment.extend_from_slice(b"<!--");
    comment.resize(32 * 1024 - 3, b' ');
    comment.extend_from_slice(b"-->");
    comment
}

fn write_worksheet(writer: &mut impl Write, comments: usize) -> u64 {
    writer.write_all(SHEET_START).unwrap();
    let comment = comment();
    for _ in 0..comments {
        writer.write_all(&comment).unwrap();
    }
    let cells = b"<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row><row r=\"1000000\"><c r=\"XFD1000000\" t=\"s\"><v>1</v></c></row>";
    writer.write_all(cells).unwrap();
    writer.write_all(SHEET_END).unwrap();
    SHEET_START.len() as u64
        + (comments * comment.len()) as u64
        + cells.len() as u64
        + SHEET_END.len() as u64
}

/// ZIP 8.6's writer accepts DEFLATE levels 1..=9. Preserve the real level-zero
/// stream using the already pinned flate2 encoder, then raw-copy its ZIP member.
/// Only this small container envelope is assembled here; ZIP independently
/// checks the complete decoded length and CRC before the service ever sees it.
fn level_zero_worksheet(comments: usize) -> crate::source::Tape {
    let encoder =
        flate2::write::DeflateEncoder::new(TapeWriter::default(), flate2::Compression::none());
    let mut checked = flate2::CrcWriter::new(encoder);
    let uncompressed = write_worksheet(&mut checked, comments);
    assert_eq!(u64::from(checked.crc().amount()), uncompressed);
    let crc = checked.crc().sum();
    let encoded = checked.into_inner().finish().unwrap().finish();
    let compressed = u32::try_from(encoded.len()).unwrap();
    let size = u32::try_from(uncompressed).unwrap();
    let name_length = u16::try_from(SHEET_NAME.len()).unwrap();
    let word = |bytes: &mut [u8], offset: usize, value: u16| {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    };
    let dword = |bytes: &mut [u8], offset: usize, value: u32| {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    let mut local = [0u8; 30];
    dword(&mut local, 0, 0x0403_4b50);
    word(&mut local, 4, 20);
    word(&mut local, 8, 8); // Actual ZIP method 8, never ZIP Stored/method 0.
    word(&mut local, 12, 33); // 1980-01-01.
    dword(&mut local, 14, crc);
    dword(&mut local, 18, compressed);
    dword(&mut local, 22, size);
    word(&mut local, 26, name_length);
    let mut writer = TapeWriter::default();
    writer.write_all(&local).unwrap();
    writer.write_all(SHEET_NAME.as_bytes()).unwrap();
    assert_eq!(
        io::copy(&mut encoded.reader(), &mut writer).unwrap(),
        encoded.len()
    );
    let directory_start = u32::try_from(local.len() + SHEET_NAME.len()).unwrap() + compressed;
    let mut central = [0u8; 46];
    dword(&mut central, 0, 0x0201_4b50);
    word(&mut central, 4, 20);
    central[6..30].copy_from_slice(&local[4..28]);
    writer.write_all(&central).unwrap();
    writer.write_all(SHEET_NAME.as_bytes()).unwrap();
    let mut end = [0u8; 22];
    dword(&mut end, 0, 0x0605_4b50);
    word(&mut end, 8, 1);
    word(&mut end, 10, 1);
    dword(
        &mut end,
        12,
        u32::try_from(central.len() + SHEET_NAME.len()).unwrap(),
    );
    dword(&mut end, 16, directory_start);
    writer.write_all(&end).unwrap();
    let tape = writer.finish();
    // The encoder's genuine method-8 data remains compact in the run-length tape.
    let mut archive = ZipArchive::new(tape.reader()).unwrap();
    let mut file = archive.by_name(SHEET_NAME).unwrap();
    assert_eq!(file.compression(), CompressionMethod::Deflated);
    assert_eq!(file.compressed_size(), u64::from(compressed));
    assert_eq!(file.size(), uncompressed);
    assert_eq!(io::copy(&mut file, &mut io::sink()).unwrap(), uncompressed);
    drop(file);
    drop(archive);
    tape
}

fn make(entries: usize, target_name_bytes: Option<usize>, comments: usize, level: i64) -> Fixture {
    assert!((PARTS.len() + 1..=10_000).contains(&entries));
    let mut writer = ZipWriter::new(TapeWriter::default());
    for (name, xml) in PARTS {
        writer.start_file(name, options(1)).unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
    }
    let uncompressed = if level == 0 {
        let raw = level_zero_worksheet(comments);
        let mut archive = ZipArchive::new(raw.reader()).unwrap();
        let file = archive.by_name(SHEET_NAME).unwrap();
        let uncompressed = file.size();
        writer.raw_copy_file(file).unwrap();
        uncompressed
    } else {
        writer.start_file(SHEET_NAME, options(level)).unwrap();
        write_worksheet(&mut writer, comments)
    };

    let base_names: usize =
        PARTS.iter().map(|(name, _)| name.len()).sum::<usize>() + SHEET_NAME.len();
    let filler_count = entries - PARTS.len() - 1;
    if filler_count > 0 {
        let remaining = target_name_bytes.unwrap() - base_names;
        let short = remaining.checked_div(filler_count).unwrap();
        let extra = remaining % filler_count;
        for index in 0..filler_count {
            let prefix = format!("unused/{index:05}/");
            let size = short + usize::from(index < extra);
            assert!((prefix.len() + 4..=4096).contains(&size));
            let name = format!("{prefix}{}.bin", "x".repeat(size - prefix.len() - 4));
            writer.start_file(name, options(1)).unwrap();
            // Even unused zero-length members are real DEFLATE members, not method-0 stubs.
            writer.write_all(b"").unwrap();
        }
    }
    let tape = Arc::new(writer.finish().unwrap().finish());
    // Verify actual written directory/method/bounds before any allocation baseline.
    let (actual_entries, actual_names, worksheet, shared_strings) = {
        let mut archive = ZipArchive::new(tape.reader()).unwrap();
        let actual_entries = archive.len();
        let actual_names: usize = archive.file_names().map(str::len).sum();
        let file = archive.by_name(SHEET_NAME).unwrap();
        assert_eq!(file.compression(), CompressionMethod::Deflated);
        assert_eq!(file.size(), uncompressed);
        let start = file
            .data_start()
            .expect("generated worksheet has a compressed data offset");
        let range = start..start + file.compressed_size();
        drop(file);
        let shared = archive.by_name(SHARED_STRINGS_NAME).unwrap();
        assert_eq!(shared.compression(), CompressionMethod::Deflated);
        let shared_start = shared
            .data_start()
            .expect("generated shared strings have a compressed data offset");
        let shared_range = shared_start..shared_start + shared.compressed_size();
        (actual_entries, actual_names, range, shared_range)
    };
    assert_eq!(actual_entries, entries);
    if let Some(names) = target_name_bytes {
        assert_eq!(actual_names, names);
    }
    Fixture {
        tape,
        worksheet,
        shared_strings,
        worksheet_uncompressed: uncompressed,
        entries: actual_entries,
        cumulative_name_bytes: actual_names,
    }
}

pub fn replay() -> Fixture {
    make(7, None, 128, 1)
}

pub fn ten_thousand_entries() -> Fixture {
    make(10_000, Some(1024 * 1024 - 64), 128, 1)
}

pub fn combined_work_limit() -> Fixture {
    // Actual DEFLATE level zero keeps encoded and decoded work independently below
    // 256 MiB while their SUM exceeds it. Tape stores repeated encoded runs compactly.
    let fixture = make(7, None, 130 * 1024 * 1024 / (32 * 1024), 0);
    let encoded = fixture.worksheet.end - fixture.worksheet.start;
    assert!(encoded < 256 * 1024 * 1024);
    assert!(fixture.worksheet_uncompressed < 256 * 1024 * 1024);
    assert!(encoded + fixture.worksheet_uncompressed > 256 * 1024 * 1024);
    fixture
}
