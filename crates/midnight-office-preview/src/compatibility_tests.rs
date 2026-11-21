//! Package shapes that Microsoft Office, LibreOffice and other writers produce:
//! extension markup, chart sheets, object-only pages, scripts, templates and the
//! plain-language reasons of the refusals that remain. All fixtures are synthetic.
use crate::{
    Cancellation, OfficeError, OfficeFormat, OfficeSession, OfficeSource, OfficeTarget, SheetId,
    SlideId,
};
use std::{
    io::{self, Cursor, Write},
    sync::Arc,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

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
fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in parts {
        let method = if *name == "mimetype" {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn open(parts: &[(&str, &str)], format: OfficeFormat) -> Result<OfficeSession, OfficeError> {
    OfficeSession::open(
        Arc::new(Source(package(parts))),
        format,
        &Cancellation::default(),
    )
}
fn opened(parts: &[(&str, &str)], format: OfficeFormat) -> OfficeSession {
    open(parts, format).unwrap_or_else(|error| panic!("{}", error.details()))
}
fn page(session: &mut OfficeSession, target: OfficeTarget) -> String {
    let page = session
        .render(target, &Cancellation::default())
        .unwrap_or_else(|error| panic!("{}", error.details()));
    String::from_utf8(page.bytes.to_vec()).unwrap()
}
fn refusal(result: Result<OfficeSession, OfficeError>) -> OfficeError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("the package opened"),
    }
}

const RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const OBJECTS_NOTICE: &str = "are not shown";

const XLSX_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/chartsheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#;
const XLSX_ROOT: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
/// Excel 365 workbook markup: compatibility, revision and extension elements.
const EXCEL_WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x15 xr xr6 xr10 xr2" xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xmlns:xr6="http://schemas.microsoft.com/office/spreadsheetml/2016/revision6" xmlns:xr10="http://schemas.microsoft.com/office/spreadsheetml/2016/revision10" xmlns:xr2="http://schemas.microsoft.com/office/spreadsheetml/2015/revision2"><fileVersion appName="xl" lastEdited="7" lowestEdited="7" rupBuild="28526"/><workbookPr defaultThemeVersion="202300"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="x15"><x15ac:absPath url="C:\Reports\" xmlns:x15ac="http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"/></mc:Choice></mc:AlternateContent><xr:revisionPtr revIDLastSave="0" documentId="8_{11111111-2222-3333-4444-555555555555}" xr6:coauthVersionLast="47" xr6:coauthVersionMax="47" xr10:uidLastSave="{00000000-0000-0000-0000-000000000000}"/><bookViews><workbookView xWindow="-120" yWindow="-120" windowWidth="29040" windowHeight="15720" xr2:uid="{00000000-000D-0000-FFFF-FFFF00000000}"/></bookViews><sheets><sheet name="Chart1" sheetId="2" r:id="rId2"/><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="1">Data!$A$1:$B$2</definedName></definedNames><calcPr calcId="191029"/><extLst><ext uri="{140A7094-0E35-4892-8432-C4D2E57EDEB5}" xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"><x15:workbookPr chartTrackingRefBase="1"/></ext></extLst></workbook>"#;
const EXCEL_WORKBOOK_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
/// An Excel worksheet with views, formatting, a chart, comments, a form control,
/// a table and extension conditional formatting around its cells.
const EXCEL_WORKSHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac xr xr2 xr3" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xmlns:xr2="http://schemas.microsoft.com/office/spreadsheetml/2015/revision2" xmlns:xr3="http://schemas.microsoft.com/office/spreadsheetml/2016/revision3" xr:uid="{00000000-0001-0000-0000-000000000000}"><sheetPr><tabColor rgb="FF00B050"/></sheetPr><dimension ref="A1:B2"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"><selection activeCell="B2" sqref="B2"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15" x14ac:dyDescent="0.25"/><cols><col min="1" max="1" width="20.7109375" customWidth="1"/></cols><sheetData><row r="1" spans="1:2" x14ac:dyDescent="0.25"><c r="A1" t="inlineStr"><is><t>Region</t></is></c><c r="B1" t="s"><v>0</v></c></row><row r="2" spans="1:2" x14ac:dyDescent="0.25"><c r="A2" t="inlineStr"><is><t>North</t></is></c><c r="B2"><v>42</v></c></row></sheetData><conditionalFormatting sqref="B2"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{B025F937-C7B1-47D3-B67F-A62EFF666E3E}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:id>{6A0B7C62-0000-4000-8000-000000000001}</x14:id></ext></extLst></cfRule></conditionalFormatting><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup paperSize="9" orientation="portrait"/><drawing r:id="rId1"/><legacyDrawing r:id="rId2"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="x14"><controls><mc:AlternateContent><mc:Choice Requires="x14"><control shapeId="1025" r:id="rId3" name="Check Box 1"/></mc:Choice></mc:AlternateContent></controls></mc:Choice></mc:AlternateContent><tableParts count="1"><tablePart r:id="rId4"/></tableParts><extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:cfRule type="dataBar" id="{6A0B7C62-0000-4000-8000-000000000001}"><x14:dataBar minLength="0" maxLength="100"/></x14:cfRule><xm:sqref>B2</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst></worksheet>"#;
const EXCEL_CHARTSHEET: &str = r#"<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetPr/><sheetViews><sheetView zoomScale="115" workbookViewId="0" zoomToFit="1"/></sheetViews><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><drawing r:id="rId1"/></chartsheet>"#;
const EXCEL_SHARED: &str = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><t>Total</t></si><extLst><ext uri="{11111111-1111-4111-8111-111111111111}"/></extLst></sst>"#;

fn xlsx_parts<'a>(workbook: &'a str, sheet: &'a str) -> Vec<(&'static str, &'a str)> {
    vec![
        ("[Content_Types].xml", XLSX_TYPES),
        ("_rels/.rels", XLSX_ROOT),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", EXCEL_WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/chartsheets/sheet1.xml", EXCEL_CHARTSHEET),
        ("xl/sharedStrings.xml", EXCEL_SHARED),
    ]
}

#[test]
fn excel_365_workbook_with_extensions_and_a_chart_sheet_previews() {
    let mut session = opened(
        &xlsx_parts(EXCEL_WORKBOOK, EXCEL_WORKSHEET),
        OfficeFormat::Xlsx,
    );
    let names: Vec<_> = session
        .catalog()
        .sheets
        .iter()
        .map(|sheet| sheet.name.as_str())
        .collect();
    assert_eq!(names, ["Chart1", "Data"]);
    let initial = session.initial_target();
    assert_eq!(initial, OfficeTarget::SheetFirst { sheet: SheetId(0) });
    let chart = page(&mut session, initial);
    assert!(chart.contains("This sheet is a chart, which the preview does not show."));
    assert!(chart.contains("/office/v1/sheet/1/first.html"));
    let data = page(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(1) });
    for text in ["Region", "Total", "North", "42", OBJECTS_NOTICE] {
        assert!(data.contains(text), "{text}");
    }
    assert!(!data.contains("Check Box"));
}

#[test]
fn worksheet_markup_outside_known_cells_never_shows_as_cell_text() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><x:wrap xmlns:x="urn:example:wrapper"><row r="1"><c r="A1" t="inlineStr"><is><t>Forged</t></is></c></row></x:wrap><row r="2"><c r="A2" t="inlineStr"><is><t>Real</t><extLst/></is><extLst/></c></row></sheetData></worksheet>"#;
    let workbook = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let mut session = opened(&xlsx_parts(workbook, sheet), OfficeFormat::Xlsx);
    let data = page(&mut session, OfficeTarget::SheetFirst { sheet: SheetId(0) });
    assert!(data.contains("Real") && data.contains(OBJECTS_NOTICE));
    assert!(!data.contains("Forged"));
}

const PPTX_TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#;

fn pptx(first: &str, second: &str) -> Vec<(&'static str, String)> {
    vec![
        ("[Content_Types].xml", PPTX_TYPES.to_owned()),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELS}/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#
            ),
        ),
        (
            "ppt/presentation.xml",
            format!(
                r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{RELS}"><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="{RELS}/slide" Target="slides/slide1.xml"/><Relationship Id="rId3" Type="{RELS}/slide" Target="slides/slide2.xml"/></Relationships>"#
            ),
        ),
        ("ppt/slides/slide1.xml", first.to_owned()),
        ("ppt/slides/slide2.xml", second.to_owned()),
    ]
}
fn borrowed<'a>(parts: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    parts
        .iter()
        .map(|(name, text)| (*name, text.as_str()))
        .collect()
}
const PICTURE_SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:pic><p:nvPicPr><p:cNvPr id="4" name="Picture 3"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="12192000" cy="6858000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#;
/// PowerPoint 365 slide markup: creation IDs in extension lists and a transition.
const POWERPOINT_SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"><a:extLst><a:ext uri="{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}"><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{3B1E0C5D-0000-4000-8000-000000000002}"/></a:ext></a:extLst></p:cNvPr><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="cs-CZ" dirty="0"/><a:t>Quarterly structure</a:t></a:r><a:endParaRPr lang="cs-CZ" dirty="0"/></a:p></p:txBody></p:sp></p:spTree><p:extLst><p:ext uri="{BB962C8B-B14F-4D97-AF65-F5344CB8AC3E}"><p14:creationId xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" val="2876543210"/></p:ext></p:extLst></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" Requires="p14"><p:transition spd="slow" p14:dur="2000"><p14:prism/></p:transition></mc:Choice><mc:Fallback><p:transition spd="slow"><p:fade/></p:transition></mc:Fallback></mc:AlternateContent></p:sld>"#;

#[test]
fn picture_only_first_slide_opens_with_a_notice_instead_of_failing() {
    let parts = pptx(PICTURE_SLIDE, POWERPOINT_SLIDE);
    let mut session = opened(&borrowed(&parts), OfficeFormat::Pptx);
    let initial = session.initial_target();
    let first = page(&mut session, initial);
    assert!(first.contains(
        "This slide contains only pictures, charts or other objects that the preview does not \
         show."
    ));
    assert!(!first.contains("Empty content window"));
    assert!(first.contains("/office/v1/slide/1/block/0.html"));
}

#[test]
fn powerpoint_extension_markup_does_not_claim_omitted_objects() {
    let parts = pptx(POWERPOINT_SLIDE, PICTURE_SLIDE);
    let mut session = opened(&borrowed(&parts), OfficeFormat::Pptx);
    let first = page(
        &mut session,
        OfficeTarget::SlideWindow {
            slide: SlideId(0),
            block: 0,
        },
    );
    assert!(first.contains("Quarterly structure"));
    assert!(!first.contains("omitted"), "{first}");
}

const ODF_NAMESPACES: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:form="urn:oasis:names:tc:opendocument:xmlns:form:1.0" xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" office:version="1.3""#;

/// A LibreOffice package: manifest entries for settings, thumbnail and `extra`.
fn odf(media: &str, extra: &str, content: &str) -> (String, String, String) {
    let manifest = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="{media}"/><manifest:file-entry manifest:full-path="Configurations2/" manifest:media-type="application/vnd.sun.xml.ui.configuration"/>{extra}<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Thumbnails/thumbnail.png" manifest:media-type="image/png"/></manifest:manifest>"#
    );
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {ODF_NAMESPACES}><office:scripts/><office:font-face-decls/><office:automatic-styles/><office:body>{content}</office:body></office:document-content>"#
    );
    (media.to_owned(), manifest, content)
}
fn odf_parts(odf: &(String, String, String)) -> Vec<(&'static str, &str)> {
    vec![
        ("mimetype", odf.0.as_str()),
        ("META-INF/manifest.xml", odf.1.as_str()),
        ("content.xml", odf.2.as_str()),
    ]
}
fn document_page(session: &mut OfficeSession) -> String {
    let target = session.initial_target();
    page(session, target)
}

#[test]
fn libreoffice_writer_declarations_do_not_claim_omitted_objects() {
    let text = odf(
        "application/vnd.oasis.opendocument.text",
        "",
        r#"<office:text><office:forms form:automatic-focus="false" form:apply-design-mode="false"/><text:sequence-decls><text:sequence-decl text:display-outline-level="0" text:name="Illustration"/><text:sequence-decl text:display-outline-level="0" text:name="Table"/></text:sequence-decls><text:p text:style-name="Standard">Writer paragraph</text:p></office:text>"#,
    );
    let mut session = opened(&odf_parts(&text), OfficeFormat::Odt);
    let page = document_page(&mut session);
    assert!(page.contains("Writer paragraph"));
    assert!(!page.contains("omitted"), "{page}");
}

#[test]
fn libreoffice_calc_with_macros_forms_chart_and_comment_previews_values() {
    let basic = r#"<manifest:file-entry manifest:full-path="Basic/Standard/Module1.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/Standard/script-lb.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/script-lc.xml" manifest:media-type="text/xml"/>"#;
    let spreadsheet = odf(
        "application/vnd.oasis.opendocument.spreadsheet",
        basic,
        r#"<office:spreadsheet><table:calculation-settings table:automatic-find-labels="false" table:use-regular-expressions="false" table:use-wildcards="true"/><table:content-validations/><table:table table:name="Revenue" table:style-name="ta1"><office:forms form:automatic-focus="false" form:apply-design-mode="false"/><table:shapes><draw:frame draw:z-index="0" draw:name="Chart 1"><draw:object xlink:href="./Object 1" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame></table:shapes><table:table-column table:style-name="co1" table:number-columns-repeated="2" table:default-cell-style-name="Default"/><table:table-row table:style-name="ro1"><table:table-cell office:value-type="string" calcext:value-type="string"><office:annotation draw:style-name="gr1"><dc:date>2026-11-21T09:00:00</dc:date><text:p>Reviewer note</text:p></office:annotation><text:p>Quarter</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="1250" calcext:value-type="float"><text:p>1 250</text:p></table:table-cell></table:table-row><calcext:conditional-formats><calcext:conditional-format calcext:target-range-address="Revenue.B1"><calcext:condition calcext:apply-style-name="Good" calcext:value="&gt;1000" calcext:base-cell-address="Revenue.B1"/></calcext:conditional-format></calcext:conditional-formats></table:table><table:named-expressions/><table:database-ranges/></office:spreadsheet>"#,
    );
    let mut session = opened(&odf_parts(&spreadsheet), OfficeFormat::Ods);
    let sheet = document_page(&mut session);
    for text in ["Revenue", "Quarter", "1 250", OBJECTS_NOTICE] {
        assert!(sheet.contains(text), "{text}");
    }
    assert!(!sheet.contains("Reviewer note"));
}

#[test]
fn opendocument_templates_open_like_their_documents() {
    let template = odf(
        "application/vnd.oasis.opendocument.text-template",
        "",
        r#"<office:text><text:p>Template paragraph</text:p></office:text>"#,
    );
    let mut session = opened(&odf_parts(&template), OfficeFormat::Odt);
    assert!(document_page(&mut session).contains("Template paragraph"));
}

fn docx_parts(types: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "[Content_Types].xml",
            format!(
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{types}</Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELS}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Body text</w:t></w:r></w:p></w:body></w:document>"#.to_owned(),
        ),
        ("word/vbaProject.bin", "not a VBA project".to_owned()),
    ]
}

#[test]
fn macro_and_template_main_parts_open_and_embedded_macro_workbooks_do_not_refuse() {
    for types in [
        r#"<Default Extension="xlsm" ContentType="application/vnd.ms-excel.sheet.macroEnabled.12"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.ms-word.document.macroEnabled.main+xml"/><Override PartName="/word/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/>"#,
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"/>"#,
    ] {
        let parts = docx_parts(types);
        let mut session = opened(&borrowed(&parts), OfficeFormat::Docx);
        assert!(document_page(&mut session).contains("Body text"), "{types}");
    }
}

#[test]
fn remaining_refusals_name_the_document_and_a_plain_reason() {
    let mut compound = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    compound.resize(4096, 0);
    let error = refusal(OfficeSession::open(
        Arc::new(Source(compound)),
        OfficeFormat::Pptx,
        &Cancellation::default(),
    ));
    assert_eq!(
        error.message(OfficeFormat::Pptx),
        "This presentation is password-protected or saved in the older binary Office format \
         (.doc, .xls or .ppt), which the preview cannot open."
    );
    let parts = docx_parts(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    );
    let error = refusal(open(&borrowed(&parts), OfficeFormat::Pptx));
    assert!(
        matches!(error, OfficeError::Malformed(_)),
        "{}",
        error.details()
    );
    assert_eq!(
        error.message(OfficeFormat::Pptx),
        "This presentation is damaged or is not a valid PowerPoint (.pptx) file, so the preview \
         cannot show it."
    );
    let binary = docx_parts(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.ms-excel.sheet.binary.macroEnabled.main"/>"#,
    );
    let error = refusal(open(&borrowed(&binary), OfficeFormat::Xlsx));
    assert_eq!(
        error.message(OfficeFormat::Xlsx),
        "This workbook is a binary Excel workbook (.xlsb), which the preview cannot open."
    );
    // Compression method 12 (bzip2) in the central and local headers.
    let mut bytes = package(&[("word/document.xml", "text")]);
    for signature in [b"PK\x03\x04".as_slice(), b"PK\x01\x02"] {
        let at = bytes
            .windows(4)
            .position(|magic| magic == signature)
            .unwrap();
        let method = at + if signature == b"PK\x03\x04" { 8 } else { 10 };
        bytes[method..method + 2].copy_from_slice(&12u16.to_le_bytes());
    }
    let error = refusal(OfficeSession::open(
        Arc::new(Source(bytes)),
        OfficeFormat::Docx,
        &Cancellation::default(),
    ));
    assert_eq!(
        error.message(OfficeFormat::Docx),
        "This document uses a compression method other than Deflate, which the preview cannot \
         read."
    );
    assert_eq!(
        OfficeError::Encrypted.message(OfficeFormat::Docx),
        "This document is password-protected, so the preview cannot open it."
    );
    struct Oversized;
    impl OfficeSource for Oversized {
        fn len(&self) -> u64 {
            256 * 1024 * 1024 + 1
        }
        fn read_at(&self, _: u64, _: &mut [u8], _: &Cancellation) -> io::Result<usize> {
            unreachable!("an oversized container is refused before any read")
        }
    }
    let error = refusal(OfficeSession::open(
        Arc::new(Oversized),
        OfficeFormat::Pptx,
        &Cancellation::default(),
    ));
    assert_eq!(
        error.message(OfficeFormat::Pptx),
        "This presentation is larger than 256 MiB, more than the preview opens."
    );
    assert_eq!(
        error.details(),
        "Office preview limit exceeded: encoded container"
    );
}
