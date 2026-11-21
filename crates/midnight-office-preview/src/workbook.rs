//! SpreadsheetML workbook catalog, and the OOXML package entry shared by DOCX, XLSX
//! and PPTX: package relationships, content types and the main part.
use crate::{
    OfficeCatalog, OfficeError, OfficeFormat, SheetDescriptor, SheetId,
    package::{Package, relationship_target},
    xml::{Element, Namespace, Token},
};
use std::collections::BTreeSet;

/// What a catalog sheet holds: worksheet cells, or a sheet the preview only names.
pub(crate) enum SheetPart {
    Cells(String),
    /// Completes "This sheet …" on the sheet's page.
    Unshown(&'static str),
}

pub(crate) struct Workbook {
    pub catalog: OfficeCatalog,
    pub parts: Vec<SheetPart>,
    pub shared_strings: Option<String>,
}
pub(crate) struct Relationship {
    pub id: String,
    pub kind: String,
    pub target: Option<String>,
}

/// Main-part content types per family: document, template and macro-enabled
/// variants, and shows for presentations. The preview never reads a VBA project.
pub(crate) const WORDPROCESSING: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
    "application/vnd.ms-word.document.macroEnabled.main+xml",
    "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
];
pub(crate) const SPREADSHEET: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
    "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
    "application/vnd.ms-excel.template.macroEnabled.main+xml",
];
pub(crate) const PRESENTATION: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml",
    "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml",
    "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml",
    "application/vnd.ms-powerpoint.slideshow.macroEnabled.main+xml",
    "application/vnd.ms-powerpoint.template.macroEnabled.main+xml",
];
const BINARY_WORKBOOK: &str = "application/vnd.ms-excel.sheet.binary.macroEnabled.main";
const XLSB: &str = "is a binary Excel workbook (.xlsb), which the preview cannot open";

const CHART_SHEET: &str = "is a chart, which the preview does not show";
const DIALOG_SHEET: &str = "is a dialog sheet, which the preview does not show";
const MACRO_SHEET: &str = "is a macro sheet, which the preview does not show";
const EXTERNAL_SHEET: &str = "is stored outside this workbook, which the preview does not open";
const OTHER_SHEET: &str = "is a kind of sheet that the preview does not show";

impl Workbook {
    pub fn load(package: &mut Package) -> Result<Self, OfficeError> {
        let workbook_part = office_part(package, SPREADSHEET)?;
        let workbook = workbook_part.as_str();
        let (directory, file) = workbook.rsplit_once('/').unwrap_or(("", workbook));
        let relation_path = if directory.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{directory}/_rels/{file}.rels")
        };
        let relations = relationships(package, &relation_path, workbook)?;
        let mut shared = relations.iter().filter(|r| kind(&r.kind, "sharedStrings"));
        let shared_strings = shared.next().and_then(|r| r.target.clone());
        if shared.next().is_some() {
            return Err(OfficeError::Malformed("multiple shared-string tables"));
        }
        let mut xml = package.xml(workbook)?;
        let mut sheets = Vec::new();
        let mut parts = Vec::new();
        let mut ids = BTreeSet::new();
        let mut sheet_ids = BTreeSet::new();
        // Only workbook/sheets/sheet is read. Views, names, calculation settings,
        // extension lists and other writers' markup are skipped whole.
        let mut depth = 0usize;
        let mut skipped = 0usize;
        let mut sheets_seen = false;
        loop {
            match xml.next()? {
                Token::Start(element) => {
                    if skipped > 0 {
                        skipped += 1;
                        continue;
                    }
                    match depth {
                        0 if !element.is(Namespace::Sheet, "workbook") => {
                            return Err(OfficeError::Malformed("workbook root"));
                        }
                        0 => (),
                        1 if element.is(Namespace::Sheet, "sheets") => {
                            if sheets_seen {
                                return Err(OfficeError::Malformed("workbook sheets structure"));
                            }
                            sheets_seen = true;
                        }
                        1 if element.is(Namespace::Sheet, "sheet") => {
                            return Err(OfficeError::Malformed("sheet outside sheets"));
                        }
                        2 if element.is(Namespace::Sheet, "sheet") => {
                            if sheets.len() >= 1024 {
                                return Err(OfficeError::LimitExceeded("sheet descriptors"));
                            }
                            let (name, relationship) =
                                sheet(&element, &relations, &mut ids, &mut sheet_ids)?;
                            sheets.push(SheetDescriptor {
                                id: SheetId(sheets.len() as u16),
                                name: name.to_owned(),
                            });
                            parts.push(sheet_part(relationship));
                        }
                        _ => {
                            skipped = 1;
                            continue;
                        }
                    }
                    depth += 1;
                }
                Token::End(_, _) => {
                    if skipped > 0 {
                        skipped -= 1;
                    } else {
                        depth -= 1;
                    }
                }
                Token::Eof => break,
                Token::Text(_) => (),
            }
        }
        if !sheets_seen {
            return Err(OfficeError::Malformed("workbook has no sheets element"));
        }
        drop(xml);
        let cells = parts.iter().filter_map(|part| match part {
            SheetPart::Cells(part) => Some(part),
            SheetPart::Unshown(_) => None,
        });
        for part in cells.chain(shared_strings.iter()) {
            if !package.contains(part) {
                return Err(OfficeError::Malformed("missing workbook member"));
            }
        }
        Ok(Self {
            catalog: OfficeCatalog {
                format: OfficeFormat::Xlsx,
                sheets,
                document: None,
                slides: Vec::new(),
            },
            parts,
            shared_strings,
        })
    }
}

/// Checks one `sheet` descriptor; returns its name and relationship.
fn sheet<'a, 'e>(
    element: &'e Element,
    relations: &'a [Relationship],
    ids: &mut BTreeSet<String>,
    sheet_ids: &mut BTreeSet<String>,
) -> Result<(&'e str, &'a Relationship), OfficeError> {
    let name = element.required("name")?;
    if name.len() > 1024 {
        return Err(OfficeError::LimitExceeded("sheet name"));
    }
    let id = element
        .attr_ns(Namespace::Relationships, "id")
        .ok_or(OfficeError::Malformed("sheet relationship ID"))?;
    let sheet_id = element.required("sheetId")?;
    if id.len() > 1024
        || sheet_id.len() > 10
        || !sheet_id.bytes().all(|c| c.is_ascii_digit())
        || sheet_id.parse::<u32>().ok().is_none_or(|value| value == 0)
    {
        return Err(OfficeError::Malformed("sheet identifier bounds"));
    }
    if !ids.insert(id.to_owned()) || !sheet_ids.insert(sheet_id.to_owned()) {
        return Err(OfficeError::Malformed("duplicate sheet identifier"));
    }
    let relationship = relations
        .iter()
        .find(|r| r.id == id)
        .ok_or(OfficeError::Malformed("missing worksheet relationship"))?;
    Ok((name, relationship))
}

/// Worksheets show their cells; chart, dialog, macro and external sheets keep their
/// place in the catalog, and their page says what they are.
fn sheet_part(relationship: &Relationship) -> SheetPart {
    let relation = relationship.kind.as_str();
    match &relationship.target {
        None => SheetPart::Unshown(EXTERNAL_SHEET),
        Some(target) if kind(relation, "worksheet") => SheetPart::Cells(target.clone()),
        Some(_) if kind(relation, "chartsheet") => SheetPart::Unshown(CHART_SHEET),
        Some(_) if kind(relation, "dialogsheet") => SheetPart::Unshown(DIALOG_SHEET),
        Some(_)
            if relation.ends_with("/xlMacrosheet") || relation.ends_with("/xlIntlMacrosheet") =>
        {
            SheetPart::Unshown(MACRO_SHEET)
        }
        Some(_) => SheetPart::Unshown(OTHER_SHEET),
    }
}

pub(crate) fn kind(value: &str, suffix: &str) -> bool {
    value
        .strip_prefix("http://schemas.openxmlformats.org/officeDocument/2006/relationships/")
        .or_else(|| value.strip_prefix("http://purl.oclc.org/ooxml/officeDocument/relationships/"))
        == Some(suffix)
}

pub(crate) fn relationships(
    package: &mut Package,
    path: &str,
    base: &str,
) -> Result<Vec<Relationship>, OfficeError> {
    let mut xml = package.xml(path)?;
    let mut result: Vec<Relationship> = Vec::new();
    let mut depth = 0;
    let mut retained = 0;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                depth += 1;
                if depth == 1 {
                    if !element.is(Namespace::PackageRelationships, "Relationships") {
                        return Err(OfficeError::Malformed("relationship root"));
                    }
                } else if depth == 2 && element.is(Namespace::PackageRelationships, "Relationship")
                {
                    if result.len() >= 2048 {
                        return Err(OfficeError::LimitExceeded("relationships"));
                    }
                    let id = element.required("Id")?;
                    let kind = element.required("Type")?;
                    let raw = element.required("Target")?;
                    if id.is_empty() || id.len() > 1024 || kind.len() > 512 || raw.len() > 4096 {
                        return Err(OfficeError::LimitExceeded("relationship descriptor"));
                    }
                    if result.iter().any(|r| r.id == id) {
                        return Err(OfficeError::Malformed("duplicate relationship ID"));
                    }
                    let target = match element.attr("TargetMode") {
                        Some("External") => None,
                        None | Some("Internal") => Some(relationship_target(base, raw)?),
                        _ => return Err(OfficeError::Malformed("relationship target mode")),
                    };
                    retained += id.len() + kind.len() + target.as_ref().map_or(0, String::len);
                    if retained > 256 * 1024 {
                        return Err(OfficeError::LimitExceeded("relationship storage"));
                    }
                    result.push(Relationship {
                        id: id.to_owned(),
                        kind: kind.to_owned(),
                        target,
                    });
                } else {
                    return Err(OfficeError::Malformed("relationship element"));
                }
            }
            Token::End(_, _) => depth -= 1,
            Token::Text(text) if !text.trim().is_empty() => {
                return Err(OfficeError::Malformed("relationship text"));
            }
            Token::Eof => break,
            Token::Text(_) => (),
        }
    }
    Ok(result)
}

/// The main part named by the package's officeDocument relationship, after checking
/// that its content type belongs to `family`.
pub(crate) fn office_part(package: &mut Package, family: &[&str]) -> Result<String, OfficeError> {
    let root = relationships(package, "_rels/.rels", "")?;
    let mut office = root
        .iter()
        .filter(|relation| kind(&relation.kind, "officeDocument"));
    let part = office
        .next()
        .and_then(|relation| relation.target.as_deref())
        .ok_or(OfficeError::Malformed("no internal main-part relationship"))?;
    if office.next().is_some() {
        return Err(OfficeError::Malformed("ambiguous Office document"));
    }
    content_type(package, part, family)?;
    if !package.contains(part) {
        return Err(OfficeError::Malformed("missing Office main member"));
    }
    Ok(part.to_owned())
}

/// The main part's content type is its Override, or else the Default for its
/// extension (Open Packaging Conventions; names and extensions ignore ASCII case).
/// Other parts' types, such as embedded macro-enabled workbooks, are not refused.
fn content_type(package: &mut Package, main: &str, family: &[&str]) -> Result<(), OfficeError> {
    let mut xml = package.xml("[Content_Types].xml")?;
    let extension = main.rsplit_once('.').map(|(_, extension)| extension);
    let mut overridden = None;
    let mut default = None;
    let mut names = BTreeSet::new();
    let mut retained = 0;
    let mut depth = 0;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                depth += 1;
                if depth == 1 && element.is(Namespace::ContentTypes, "Types") {
                    continue;
                }
                if depth != 2 || element.ns != Namespace::ContentTypes {
                    return Err(OfficeError::Malformed("content type structure"));
                }
                let content = element.required("ContentType")?;
                let key = match element.name.as_str() {
                    "Override" => element.required("PartName")?,
                    "Default" => element.required("Extension")?,
                    _ => return Err(OfficeError::Malformed("content type element")),
                };
                retained += key.len();
                if names.len() >= 10_000 || retained > 1024 * 1024 {
                    return Err(OfficeError::LimitExceeded("content type descriptors"));
                }
                if !names.insert(key.to_owned()) {
                    return Err(OfficeError::Malformed("duplicate content type"));
                }
                if element.name == "Override"
                    && key
                        .strip_prefix('/')
                        .is_some_and(|name| name.eq_ignore_ascii_case(main))
                {
                    overridden = Some(content.to_owned());
                } else if element.name == "Default"
                    && extension.is_some_and(|extension| extension.eq_ignore_ascii_case(key))
                {
                    default = Some(content.to_owned());
                }
            }
            Token::End(_, _) => depth -= 1,
            Token::Eof => break,
            Token::Text(_) => (),
        }
    }
    match overridden.or(default) {
        Some(content) if family.contains(&content.as_str()) => Ok(()),
        Some(content) if content == BINARY_WORKBOOK => Err(OfficeError::Unsupported(XLSB)),
        Some(_) => Err(OfficeError::Malformed(
            "main part content type of another format",
        )),
        None => Err(OfficeError::Malformed("main part without a content type")),
    }
}
