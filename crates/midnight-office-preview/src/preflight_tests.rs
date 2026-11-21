//! Hand-built ZIP containers: Office growth-hint padding, every skipped extra
//! record, ZIP64 sizes, malformed extra fields, local/central disagreements and
//! limits. All fixtures are synthetic.
use crate::{Cancellation, OfficeError, OfficeFormat, OfficeSession, OfficeSource};
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const SATURATED: u32 = u32::MAX;

struct Source {
    bytes: Vec<u8>,
    delivered: AtomicUsize,
}
impl OfficeSource for Source {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8], _: &Cancellation) -> io::Result<usize> {
        let bytes = self
            .bytes
            .get(offset as usize..)
            .ok_or_else(|| io::Error::other("offset"))?;
        let length = bytes.len().min(output.len());
        output[..length].copy_from_slice(&bytes[..length]);
        self.delivered.fetch_add(length, Ordering::Relaxed);
        Ok(length)
    }
}
fn source(bytes: Vec<u8>) -> Arc<Source> {
    Arc::new(Source {
        bytes,
        delivered: AtomicUsize::new(0),
    })
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}
/// One extra-field record: header ID, data size and data.
fn record(id: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = id.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
    bytes.extend_from_slice(data);
    bytes
}
/// Microsoft's growth hint: signature 0xA028, padding size, then the padding.
fn growth_hint(padding: u16) -> Vec<u8> {
    let mut data = vec![0x28, 0xa0];
    data.extend_from_slice(&padding.to_le_bytes());
    data.resize(4 + usize::from(padding), 0);
    record(0xa220, &data)
}
fn unicode_path(name: &[u8], crc: u32) -> Vec<u8> {
    let mut data = vec![1];
    data.extend_from_slice(&crc.to_le_bytes());
    data.extend_from_slice(name);
    record(0x7075, &data)
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Descriptor {
    #[default]
    None,
    Plain,
    Signed,
    /// Eight-byte sizes, announced by a local ZIP64 record.
    Wide,
}
/// Which 32-bit fields are saturated and supplied by ZIP64 records.
#[derive(Clone, Copy, Default)]
struct Zip64 {
    central_sizes: bool,
    central_offset: bool,
    local_sizes: bool,
}
/// One member as a ZIP writer lays it out.
#[derive(Clone, Default)]
struct Member {
    name: String,
    text: Vec<u8>,
    flags: u16,
    local_extra: Vec<u8>,
    central_extra: Vec<u8>,
    comment: Vec<u8>,
    descriptor: Descriptor,
    zip64: Zip64,
}
/// A change to one member before the container is built.
type Edit = fn(&mut Member);
/// The container and where each member's local and central headers start.
struct Built {
    bytes: Vec<u8>,
    local: Vec<usize>,
    central: Vec<usize>,
}
fn push16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Deflated members, local headers first, then the central directory and end record.
fn build(members: &[Member], archive_comment: &[u8]) -> Built {
    let mut bytes = Vec::new();
    let mut local = Vec::new();
    let mut written = Vec::new();
    for member in members {
        local.push(bytes.len());
        let data = deflate(&member.text);
        let crc = crc32(&member.text);
        let described = member.descriptor != Descriptor::None;
        let flags = member.flags | if described { 8 } else { 0 };
        let mut extra = Vec::new();
        if member.zip64.local_sizes || member.descriptor == Descriptor::Wide {
            let mut sizes = Vec::new();
            let known = u64::from(!described);
            push64(&mut sizes, member.text.len() as u64 * known);
            push64(&mut sizes, data.len() as u64 * known);
            extra.extend(record(0x0001, &sizes));
        }
        extra.extend_from_slice(&member.local_extra);
        let (crc_field, compressed, original) = if described {
            (0, 0, 0)
        } else if member.zip64.local_sizes {
            (crc, SATURATED, SATURATED)
        } else {
            (crc, data.len() as u32, member.text.len() as u32)
        };
        bytes.extend_from_slice(b"PK\x03\x04");
        push16(&mut bytes, 20);
        push16(&mut bytes, flags);
        push16(&mut bytes, 8);
        push16(&mut bytes, 0);
        push16(&mut bytes, 0x21);
        push32(&mut bytes, crc_field);
        push32(&mut bytes, compressed);
        push32(&mut bytes, original);
        push16(&mut bytes, member.name.len() as u16);
        push16(&mut bytes, extra.len() as u16);
        bytes.extend_from_slice(member.name.as_bytes());
        bytes.extend_from_slice(&extra);
        bytes.extend_from_slice(&data);
        if matches!(member.descriptor, Descriptor::Signed | Descriptor::Wide) {
            bytes.extend_from_slice(b"PK\x07\x08");
        }
        match member.descriptor {
            Descriptor::None => (),
            Descriptor::Plain | Descriptor::Signed => {
                push32(&mut bytes, crc);
                push32(&mut bytes, data.len() as u32);
                push32(&mut bytes, member.text.len() as u32);
            }
            Descriptor::Wide => {
                push32(&mut bytes, crc);
                push64(&mut bytes, data.len() as u64);
                push64(&mut bytes, member.text.len() as u64);
            }
        }
        written.push((crc, data.len(), flags));
    }
    let central_start = bytes.len();
    let mut central = Vec::new();
    for ((member, &offset), &(crc, compressed, flags)) in members.iter().zip(&local).zip(&written) {
        central.push(bytes.len());
        let mut wide = Vec::new();
        if member.zip64.central_sizes {
            push64(&mut wide, member.text.len() as u64);
            push64(&mut wide, compressed as u64);
        }
        if member.zip64.central_offset {
            push64(&mut wide, offset as u64);
        }
        let mut extra = if wide.is_empty() {
            Vec::new()
        } else {
            record(0x0001, &wide)
        };
        extra.extend_from_slice(&member.central_extra);
        let field = |saturated: bool, value: usize| {
            if saturated { SATURATED } else { value as u32 }
        };
        bytes.extend_from_slice(b"PK\x01\x02");
        push16(&mut bytes, 45);
        push16(&mut bytes, 20);
        push16(&mut bytes, flags);
        push16(&mut bytes, 8);
        push16(&mut bytes, 0);
        push16(&mut bytes, 0x21);
        push32(&mut bytes, crc);
        push32(&mut bytes, field(member.zip64.central_sizes, compressed));
        push32(
            &mut bytes,
            field(member.zip64.central_sizes, member.text.len()),
        );
        push16(&mut bytes, member.name.len() as u16);
        push16(&mut bytes, extra.len() as u16);
        push16(&mut bytes, member.comment.len() as u16);
        push16(&mut bytes, 0);
        push16(&mut bytes, 0);
        push32(&mut bytes, 0);
        push32(&mut bytes, field(member.zip64.central_offset, offset));
        bytes.extend_from_slice(member.name.as_bytes());
        bytes.extend_from_slice(&extra);
        bytes.extend_from_slice(&member.comment);
    }
    let central_size = bytes.len() - central_start;
    bytes.extend_from_slice(b"PK\x05\x06");
    push32(&mut bytes, 0);
    push16(&mut bytes, members.len() as u16);
    push16(&mut bytes, members.len() as u16);
    push32(&mut bytes, central_size as u32);
    push32(&mut bytes, central_start as u32);
    push16(&mut bytes, archive_comment.len() as u16);
    bytes.extend_from_slice(archive_comment);
    Built {
        bytes,
        local,
        central,
    }
}

const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const RELATIONSHIP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn content_types(main: &str, part: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/{part}" ContentType="application/vnd.openxmlformats-officedocument.{main}.main+xml"/></Types>"#
    )
}
fn package_relationships(part: &str) -> String {
    format!(
        r#"<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{RELATIONSHIP}/officeDocument" Target="{part}"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
    )
}

/// The parts of a small package of `format`, in Microsoft Office's member order.
fn parts(format: OfficeFormat, text: &str) -> Vec<(&'static str, String)> {
    let core = (
        "docProps/core.xml",
        r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"/>"#.to_owned(),
    );
    match format {
        OfficeFormat::Pptx => vec![
            (
                "[Content_Types].xml",
                content_types("presentationml.presentation", "ppt/presentation.xml"),
            ),
            ("_rels/.rels", package_relationships("ppt/presentation.xml")),
            (
                "ppt/presentation.xml",
                format!(
                    r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{RELATIONSHIP}"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#
                ),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                format!(
                    r#"<Relationships xmlns="{PACKAGE}"><Relationship Id="rId2" Type="{RELATIONSHIP}/slide" Target="slides/slide1.xml"/></Relationships>"#
                ),
            ),
            (
                "ppt/slides/slide1.xml",
                format!(
                    r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
                ),
            ),
            core,
        ],
        OfficeFormat::Docx => vec![
            (
                "[Content_Types].xml",
                content_types("wordprocessingml.document", "word/document.xml"),
            ),
            ("_rels/.rels", package_relationships("word/document.xml")),
            (
                "word/document.xml",
                format!(
                    r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
                ),
            ),
            (
                "word/_rels/document.xml.rels",
                format!(r#"<Relationships xmlns="{PACKAGE}"/>"#),
            ),
            core,
        ],
        _ => vec![
            (
                "[Content_Types].xml",
                content_types("spreadsheetml.sheet", "xl/workbook.xml"),
            ),
            ("_rels/.rels", package_relationships("xl/workbook.xml")),
            (
                "xl/workbook.xml",
                format!(
                    r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{RELATIONSHIP}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
                ),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                format!(
                    r#"<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{RELATIONSHIP}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
                ),
            ),
            (
                "xl/worksheets/sheet1.xml",
                format!(
                    r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{text}</t></is></c></row></sheetData></worksheet>"#
                ),
            ),
            core,
        ],
    }
}

/// Members as Microsoft Office writes them: Deflate with option bits 1 and 2, and
/// growth hints of 512 bytes on the first two parts and 256 bytes on the fourth.
fn office_members(format: OfficeFormat, text: &str) -> Vec<Member> {
    parts(format, text)
        .into_iter()
        .enumerate()
        .map(|(index, (name, xml))| Member {
            name: name.to_owned(),
            text: xml.into_bytes(),
            flags: 6,
            local_extra: match index {
                0 | 1 => growth_hint(512),
                3 => growth_hint(256),
                _ => Vec::new(),
            },
            ..Member::default()
        })
        .collect()
}

fn open(bytes: Vec<u8>, format: OfficeFormat) -> Result<OfficeSession, OfficeError> {
    OfficeSession::open(source(bytes), format, &Cancellation::default())
}
fn first_page(bytes: Vec<u8>, format: OfficeFormat) -> String {
    let mut session = open(bytes, format).unwrap_or_else(|error| panic!("{}", error.details()));
    let target = session.initial_target();
    let page = session
        .render(target, &Cancellation::default())
        .unwrap_or_else(|error| panic!("{}", error.details()));
    String::from_utf8(page.bytes.to_vec()).unwrap()
}
fn malformed(result: Result<OfficeSession, OfficeError>, detail: &str) {
    match result {
        Err(OfficeError::Malformed(reason)) if reason.contains(detail) => (),
        Err(error) => panic!("expected {detail:?}, got {:?}", error.details()),
        Ok(_) => panic!("expected {detail:?}, but the package opened"),
    }
}
fn limited(result: Result<OfficeSession, OfficeError>, limit: &str) {
    match result {
        Err(OfficeError::LimitExceeded(reason)) if reason == limit => (),
        Err(error) => panic!("expected limit {limit:?}, got {:?}", error.details()),
        Ok(_) => panic!("expected limit {limit:?}, but the package opened"),
    }
}

#[test]
fn office_growth_hint_padding_previews_presentation_document_and_workbook() {
    for format in [OfficeFormat::Pptx, OfficeFormat::Docx, OfficeFormat::Xlsx] {
        let built = build(&office_members(format, "Padded first parts"), b"");
        // The first local header holds 520 bytes of extra field, as Office writes it.
        assert_eq!(u16::from_le_bytes([built.bytes[28], built.bytes[29]]), 520);
        assert!(first_page(built.bytes, format).contains("Padded first parts"));
    }
    // No prefix of the file bounds the headers: every part padded to the largest
    // extra field still opens, and name plus padding spans more than one read chunk.
    let mut members = office_members(OfficeFormat::Pptx, "Fully padded");
    for member in &mut members {
        member.local_extra = growth_hint(65_000);
        member.central_extra = growth_hint(4_000);
    }
    let built = build(&members, b"");
    assert!(built.bytes.len() > 6 * 65_000);
    assert!(first_page(built.bytes, OfficeFormat::Pptx).contains("Fully padded"));
}

#[test]
fn every_skipped_extra_record_is_accepted_in_local_and_central_headers() {
    let mut ntfs = vec![0; 4];
    ntfs.extend(record(1, &[7; 24]));
    let skipped = [
        growth_hint(16),
        record(0x5455, &[1, 0x80, 0x5a, 0x1e, 0x66]),
        record(0x000a, &ntfs),
        record(0x7875, &[1, 4, 0xe8, 3, 0, 0, 4, 0xe8, 3, 0, 0]),
        record(0xcafe, &[]),
        record(0xd935, &[4, 0, 0, 0]),
        record(0x6b6f, &[1, 2, 3]),
    ];
    let every = skipped.concat();
    for block in skipped.iter().chain([&every]) {
        let mut members = office_members(OfficeFormat::Pptx, "Skipped records");
        assert_eq!(members[4].name, "ppt/slides/slide1.xml");
        members[4].local_extra = block.clone();
        members[4].central_extra = block.clone();
        members[4].comment = b"Entry comment".to_vec();
        let built = build(&members, b"Saved by a ZIP tool");
        assert!(first_page(built.bytes, OfficeFormat::Pptx).contains("Skipped records"));
    }
}

#[test]
fn extra_records_that_zip_would_reject_or_rename_never_reach_its_index() {
    // zip refuses an NTFS record that is not 32 bytes long and a Unicode path record
    // whose CRC is stale, and renames a member after a current one. The preflight
    // checks them; zip indexes a central directory without them.
    let mut members = office_members(OfficeFormat::Pptx, "Canonical index");
    let name = members[4].name.clone().into_bytes();
    members[4].central_extra = [
        record(0x000a, &[0; 36]),
        unicode_path(b"ppt/slides/other.xml", crc32(b"stale")),
        unicode_path(&name, crc32(&name)),
    ]
    .concat();
    let built = build(&members, b"");
    assert!(first_page(built.bytes, OfficeFormat::Pptx).contains("Canonical index"));
}

#[test]
fn zip64_records_supply_saturated_sizes_offsets_and_descriptor_widths() {
    let wide = |central_sizes, central_offset, local_sizes| Zip64 {
        central_sizes,
        central_offset,
        local_sizes,
    };
    for (zip64, descriptor) in [
        (wide(true, true, false), Descriptor::None),
        (wide(false, false, true), Descriptor::None),
        (wide(true, false, false), Descriptor::Wide),
        (Zip64::default(), Descriptor::Signed),
        (Zip64::default(), Descriptor::Plain),
    ] {
        let mut members = office_members(OfficeFormat::Docx, "ZIP64 sizes");
        for member in &mut members {
            member.zip64 = zip64;
            member.descriptor = descriptor;
        }
        let built = build(&members, b"");
        assert!(first_page(built.bytes, OfficeFormat::Docx).contains("ZIP64 sizes"));
    }
}

#[test]
fn malformed_extra_fields_are_refused_with_specific_reasons() {
    // Member 2 (word/document.xml) carries the defect; the first parts keep padding.
    let edits: [(Edit, &str); 5] = [
        (
            |member| member.local_extra = [growth_hint(8), vec![0]].concat(),
            "ZIP extra field ends inside a record header",
        ),
        (
            |member| member.central_extra = vec![0x55, 0x54, 9, 0, 1, 2, 3, 4, 5],
            "ZIP extra record overruns its field",
        ),
        (
            |member| member.central_extra = [record(0xcafe, &[]), vec![1, 0, 0]].concat(),
            "ZIP extra field ends inside a record header",
        ),
        (
            |member| {
                member.zip64.central_sizes = true;
                member.central_extra = record(0x0001, &[0; 16]);
            },
            "repeated ZIP64 extra record",
        ),
        (
            |member| {
                let name = member.name.clone().into_bytes();
                member.central_extra = unicode_path(b"word/other.xml", crc32(&name));
            },
            "ZIP Unicode path record names a different part",
        ),
    ];
    for (edit, detail) in edits {
        let mut members = office_members(OfficeFormat::Docx, "Malformed extras");
        edit(&mut members[2]);
        malformed(open(build(&members, b"").bytes, OfficeFormat::Docx), detail);
    }
    // Saturated fields that no ZIP64 record supplies: (central?, offset, detail).
    for (central, offset, detail) in [
        (true, 20, "ZIP64 record lacks the compressed size"),
        (true, 42, "ZIP64 record lacks the local header offset"),
        (false, 18, "local ZIP64 record lacks the member sizes"),
    ] {
        let built = build(&office_members(OfficeFormat::Docx, "Saturated"), b"");
        let mut bytes = built.bytes;
        let at = if central {
            built.central[2]
        } else {
            built.local[2]
        } + offset;
        bytes[at..at + 4].copy_from_slice(&SATURATED.to_le_bytes());
        malformed(open(bytes, OfficeFormat::Docx), detail);
    }
    let mut members = office_members(OfficeFormat::Docx, "Message");
    members[0].local_extra.push(0);
    let error = open(build(&members, b"").bytes, OfficeFormat::Docx)
        .err()
        .unwrap();
    assert_eq!(
        error.message(OfficeFormat::Docx),
        "This document is damaged or is not a valid Word (.docx) file, so the preview cannot \
         show it."
    );
    assert_eq!(
        error.details(),
        "Malformed Office package: ZIP extra field ends inside a record header"
    );
}

#[test]
fn local_and_central_disagreements_are_refused_beside_valid_padding() {
    // Member 1 (_rels/.rels) has 520 bytes of padding: (local field offset, bytes).
    let cases: [(usize, &[u8], &str); 5] = [
        (8, &[0, 0], "local/central header mismatch"),
        (6, &[0, 0], "local/central header mismatch"),
        (30, b"X", "local/central name mismatch"),
        (14, &[1, 2, 3, 4], "local/central size or checksum mismatch"),
        (18, &[1, 0, 0, 0], "local/central size or checksum mismatch"),
    ];
    for (offset, replacement, detail) in cases {
        let built = build(&office_members(OfficeFormat::Pptx, "Mismatch"), b"");
        let mut bytes = built.bytes;
        let at = built.local[1] + offset;
        bytes[at..at + replacement.len()].copy_from_slice(replacement);
        malformed(open(bytes, OfficeFormat::Pptx), detail);
    }
    let mut members = office_members(OfficeFormat::Pptx, "Descriptor");
    members[1].descriptor = Descriptor::Signed;
    let built = build(&members, b"");
    let mut bytes = built.bytes;
    // The descriptor's CRC, compressed and original sizes end where member 2 starts.
    bytes[built.local[2] - 12] ^= 0xff;
    malformed(
        open(bytes, OfficeFormat::Pptx),
        "ZIP data descriptor mismatch",
    );
}

#[test]
fn overlapping_or_escaping_members_are_refused_before_their_padding_is_read() {
    // Every later central record points into the first member's accepted padding.
    let mut members = office_members(OfficeFormat::Pptx, "Overlap");
    members[0].local_extra = growth_hint(60_000);
    let mut built = build(&members, b"");
    let padding = built.local[0] + 30 + members[0].name.len() + 8;
    for (index, &central) in built.central.iter().enumerate().skip(1) {
        let inside = (padding + index * 64) as u32;
        built.bytes[central + 42..central + 46].copy_from_slice(&inside.to_le_bytes());
    }
    let captured = source(built.bytes.clone());
    malformed(
        OfficeSession::open(
            captured.clone(),
            OfficeFormat::Pptx,
            &Cancellation::default(),
        ),
        "overlapping ZIP entries",
    );
    // Signature probe, end-record search, central directory, then the first member's
    // header, name and padding: nothing inside the padding is read a second time.
    let tail = built.bytes.len().min(22 + 65_535);
    let directory = built.bytes.len() - built.central[0];
    let first = 30 + members[0].name.len() + 60_008;
    let delivered = captured.delivered.load(Ordering::Relaxed);
    assert_eq!(delivered, 8 + tail + directory + first);
    // A local extra field that runs into the central directory.
    let built = build(&office_members(OfficeFormat::Pptx, "Escape"), b"");
    let mut bytes = built.bytes;
    let at = built.local[4] + 28;
    bytes[at..at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    malformed(
        open(bytes, OfficeFormat::Pptx),
        "member outside data region",
    );
}

#[test]
fn extra_fields_do_not_lift_the_documented_limits() {
    // A ZIP64 original size of 4 GiB, agreed by both headers, cannot be indexed.
    let mut members = office_members(OfficeFormat::Xlsx, "Limit");
    members[4].zip64.central_sizes = true;
    members[4].zip64.local_sizes = true;
    let built = build(&members, b"");
    let mut bytes = built.bytes;
    let name = members[4].name.len();
    for at in [
        built.central[4] + 46 + name + 4,
        built.local[4] + 30 + name + 4,
    ] {
        bytes[at..at + 8].copy_from_slice(&(1u64 << 32).to_le_bytes());
    }
    limited(open(bytes, OfficeFormat::Xlsx), "member size");
    // A name over 4096 bytes is refused whatever its extra fields hold.
    let mut members = office_members(OfficeFormat::Xlsx, "Limit");
    members[4].name = "n".repeat(4097);
    members[4].central_extra = growth_hint(8);
    limited(
        open(build(&members, b"").bytes, OfficeFormat::Xlsx),
        "entry name",
    );
    // The 8 MiB central-directory bound counts extra fields and comments.
    let mut members = Vec::new();
    for index in 0..70 {
        members.push(Member {
            name: format!("part{index}.xml"),
            text: b"x".to_vec(),
            central_extra: growth_hint(60_000),
            comment: vec![b'c'; 65_000],
            ..Member::default()
        });
    }
    limited(
        open(build(&members, b"").bytes, OfficeFormat::Xlsx),
        "central metadata",
    );
}

#[test]
fn zip64_end_records_are_refused_with_a_plain_reason() {
    let built = build(&office_members(OfficeFormat::Pptx, "ZIP64 end"), b"");
    let mut bytes = built.bytes;
    let end = bytes.len() - 22;
    let central_start = built.central[0];
    let mut zip64 = b"PK\x06\x06".to_vec();
    push64(&mut zip64, 44);
    push16(&mut zip64, 45);
    push16(&mut zip64, 45);
    push32(&mut zip64, 0);
    push32(&mut zip64, 0);
    push64(&mut zip64, built.central.len() as u64);
    push64(&mut zip64, built.central.len() as u64);
    push64(&mut zip64, (end - central_start) as u64);
    push64(&mut zip64, central_start as u64);
    zip64.extend_from_slice(b"PK\x06\x07");
    push32(&mut zip64, 0);
    push64(&mut zip64, end as u64);
    push32(&mut zip64, 1);
    let mut record = bytes.split_off(end);
    record[8..20].fill(0xff);
    bytes.extend(zip64);
    bytes.extend(record);
    let error = open(bytes, OfficeFormat::Pptx).err().unwrap();
    assert!(
        matches!(error, OfficeError::Unsupported(_)),
        "{}",
        error.details()
    );
    assert_eq!(
        error.message(OfficeFormat::Pptx),
        "This presentation is stored in the ZIP64 format for very large archives, which the \
         preview cannot open."
    );
}
