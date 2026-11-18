use crate::{
    OfficeCatalog, OfficeError, OfficeFormat, SheetDescriptor, SheetId,
    package::{Package, relationship_target},
    xml::{Namespace, Token},
};
use std::collections::BTreeSet;

pub(crate) struct Workbook {
    pub catalog: OfficeCatalog,
    pub parts: Vec<String>,
    pub shared_strings: Option<String>,
}
pub(crate) struct Relationship {
    pub id: String,
    pub kind: String,
    pub target: Option<String>,
}

impl Workbook {
    pub fn load(package: &mut Package) -> Result<Self, OfficeError> {
        let workbook_part = office_part(
            package,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        )?;
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
        let mut stack = Vec::new();
        let mut sheets_seen = false;
        loop {
            match xml.next()? {
                Token::Start(element) => {
                    if stack.is_empty() && !element.is(Namespace::Sheet, "workbook") {
                        return Err(OfficeError::Unsupported("not an XLSX workbook"));
                    }
                    if element.ns != Namespace::Sheet || !supported_element(&element.name) {
                        return Err(OfficeError::Unsupported("workbook extension namespace"));
                    }
                    if element.name == "sheets" {
                        if sheets_seen || stack.as_slice() != ["workbook"] {
                            return Err(OfficeError::Malformed("workbook sheets structure"));
                        }
                        sheets_seen = true;
                    }
                    if element.name == "sheet" {
                        if stack.last().map(String::as_str) != Some("sheets") {
                            return Err(OfficeError::Malformed("sheet outside sheets"));
                        }
                        if sheets.len() >= 1024 {
                            return Err(OfficeError::LimitExceeded("sheet descriptors"));
                        }
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
                        if !kind(&relationship.kind, "worksheet") {
                            return Err(OfficeError::Unsupported("non-worksheet sheet"));
                        }
                        let target = relationship
                            .target
                            .as_ref()
                            .ok_or(OfficeError::Unsupported("external worksheet"))?;
                        sheets.push(SheetDescriptor {
                            id: SheetId(sheets.len() as u16),
                            name: name.to_owned(),
                        });
                        parts.push(target.clone());
                    }
                    stack.push(element.name);
                }
                Token::End(_, _) => {
                    stack.pop();
                }
                Token::Eof => break,
                Token::Text(_) => (),
            }
        }
        if !sheets_seen {
            return Err(OfficeError::Malformed("workbook has no sheets element"));
        }
        drop(xml);
        for part in parts.iter().chain(shared_strings.iter()) {
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

fn supported_element(name: &str) -> bool {
    [
        "workbook",
        "fileVersion",
        "fileSharing",
        "workbookPr",
        "workbookProtection",
        "bookViews",
        "workbookView",
        "sheets",
        "sheet",
        "functionGroups",
        "functionGroup",
        "externalReferences",
        "externalReference",
        "definedNames",
        "definedName",
        "calcPr",
        "oleSize",
        "customWorkbookViews",
        "customWorkbookView",
        "pivotCaches",
        "pivotCache",
        "smartTagPr",
        "smartTagTypes",
        "smartTagType",
        "webPublishing",
        "fileRecoveryPr",
        "webPublishObjects",
        "webPublishObject",
    ]
    .contains(&name)
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

pub(crate) fn office_part(package: &mut Package, expected: &str) -> Result<String, OfficeError> {
    let root = relationships(package, "_rels/.rels", "")?;
    let mut office = root
        .iter()
        .filter(|relation| kind(&relation.kind, "officeDocument"));
    let part = office
        .next()
        .and_then(|relation| relation.target.as_deref())
        .ok_or(OfficeError::Unsupported(
            "missing internal Office document relationship",
        ))?;
    if office.next().is_some() {
        return Err(OfficeError::Malformed("ambiguous Office document"));
    }
    content_type(package, part, expected)?;
    if !package.contains(part) {
        return Err(OfficeError::Malformed("missing Office main member"));
    }
    Ok(part.to_owned())
}

fn content_type(package: &mut Package, workbook: &str, expected: &str) -> Result<(), OfficeError> {
    let mut xml = package.xml("[Content_Types].xml")?;
    let mut found = false;
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
                if content.contains("macroEnabled") || content.contains("vbaProject") {
                    return Err(OfficeError::Unsupported("macro-enabled Office package"));
                }
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
                if element.name == "Override" && key.strip_prefix('/') == Some(workbook) {
                    if content != expected {
                        return Err(OfficeError::Unsupported("main document content type"));
                    }
                    found = true;
                }
            }
            Token::End(_, _) => depth -= 1,
            Token::Eof => break,
            Token::Text(_) => (),
        }
    }
    if !found {
        return Err(OfficeError::Unsupported("Office main content type missing"));
    }
    Ok(())
}
