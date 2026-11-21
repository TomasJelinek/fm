use std::{fmt, io, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfficeFormat {
    Xlsx,
    Docx,
    Pptx,
    Odt,
    Ods,
    Odp,
}

/// Zero-based index in this session's catalog, never a document-supplied sheetId.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SheetId(pub u16);

/// Main document identity local to this session; currently ID zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct DocumentId(pub u16);
/// Zero-based occurrence in this session's ordered slide catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SlideId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum OfficeTarget {
    Catalog,
    /// Zero-based logical paragraph/table-row window, not a print page.
    DocumentWindow {
        document: DocumentId,
        block: u32,
    },
    /// Zero-based logical paragraph/table-row window within one slide.
    SlideWindow {
        slide: SlideId,
        block: u32,
    },
    SheetFirst {
        sheet: SheetId,
    },
    /// One-based logical coordinates. At most 128 by 32 cells are materialized.
    SheetWindow {
        sheet: SheetId,
        row: u32,
        column: u16,
    },
}

impl OfficeTarget {
    /// Canonical root URL; callers must validate target membership before serving.
    pub fn path(self) -> String {
        match self {
            Self::Catalog => "/office/v1/catalog.html".to_owned(),
            Self::DocumentWindow { document, block } => {
                format!("/office/v1/document/{}/block/{block}.html", document.0)
            }
            Self::SlideWindow { slide, block } => {
                format!("/office/v1/slide/{}/block/{block}.html", slide.0)
            }
            Self::SheetFirst { sheet } => format!("/office/v1/sheet/{}/first.html", sheet.0),
            Self::SheetWindow { sheet, row, column } => {
                format!("/office/v1/sheet/{}/row/{row}/col/{column}.html", sheet.0)
            }
        }
    }

    pub(crate) fn validate(self) -> Result<(), OfficeError> {
        if let Self::DocumentWindow { block, .. } | Self::SlideWindow { block, .. } = self
            && block >= 1_048_576
        {
            return Err(OfficeError::InvalidTarget);
        }
        if let Self::SheetWindow { row, column, .. } = self
            && (!(1..=1_048_576).contains(&row) || !(1..=16_384).contains(&column))
        {
            return Err(OfficeError::InvalidTarget);
        }
        Ok(())
    }

    pub(crate) fn parse(path: &str) -> Result<Self, OfficeError> {
        if path == "/office/v1/catalog.html" {
            return Ok(Self::Catalog);
        }
        if path.len() > 100 {
            return Err(OfficeError::InvalidTarget);
        }
        let segments: Vec<_> = path.split('/').collect();
        let target = match segments.as_slice() {
            ["", "office", "v1", "document", document, "block", block] => Self::DocumentWindow {
                document: DocumentId(number(document)?),
                block: number(
                    block
                        .strip_suffix(".html")
                        .ok_or(OfficeError::InvalidTarget)?,
                )?,
            },
            ["", "office", "v1", "slide", slide, "block", block] => Self::SlideWindow {
                slide: SlideId(number(slide)?),
                block: number(
                    block
                        .strip_suffix(".html")
                        .ok_or(OfficeError::InvalidTarget)?,
                )?,
            },
            ["", "office", "v1", "sheet", sheet, "first.html"] => Self::SheetFirst {
                sheet: SheetId(number(sheet)?),
            },
            [
                "",
                "office",
                "v1",
                "sheet",
                sheet,
                "row",
                row,
                "col",
                column,
            ] => Self::SheetWindow {
                sheet: SheetId(number(sheet)?),
                row: number(row)?,
                column: number(
                    column
                        .strip_suffix(".html")
                        .ok_or(OfficeError::InvalidTarget)?,
                )?,
            },
            _ => return Err(OfficeError::InvalidTarget),
        };
        target.validate()?;
        if target.path() != path {
            return Err(OfficeError::InvalidTarget);
        }
        Ok(target)
    }
}

fn number<T: std::str::FromStr>(value: &str) -> Result<T, OfficeError> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(OfficeError::InvalidTarget);
    }
    value.parse().map_err(|_| OfficeError::InvalidTarget)
}

#[derive(Clone, Debug)]
pub struct SheetDescriptor {
    pub id: SheetId,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct DocumentDescriptor {
    pub id: DocumentId,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct SlideDescriptor {
    pub id: SlideId,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct OfficeCatalog {
    pub format: OfficeFormat,
    pub sheets: Vec<SheetDescriptor>,
    pub document: Option<DocumentDescriptor>,
    pub slides: Vec<SlideDescriptor>,
}
#[derive(Clone, Debug)]
pub struct ImmutableHtml {
    pub bytes: Arc<[u8]>,
    pub target: OfficeTarget,
    pub generation: u64,
}

/// Why a package or page cannot be previewed. [`OfficeError::message`] is the
/// sentence a user is shown; [`OfficeError::details`] keeps the internal reason.
#[derive(Debug)]
pub enum OfficeError {
    Cancelled,
    /// ZIP or ODF password protection.
    Encrypted,
    /// Something the preview deliberately does not open. The text is plain language
    /// that completes a sentence about the document ("This presentation …").
    Unsupported(&'static str),
    /// A damaged or nonconforming package. The text is diagnostic detail only.
    Malformed(&'static str),
    /// One of the fixed preview limits. The text names the limit for diagnostics.
    LimitExceeded(&'static str),
    InvalidTarget,
    Source(io::Error),
}

/// Limit names that [`OfficeError::message`] explains in plain language.
pub(crate) const CONTAINER_LIMIT: &str = "encoded container";
pub(crate) const PART_COUNT_LIMIT: &str = "package entries";
pub(crate) const WORK_LIMIT: &str = "generation delivered/decompressed bytes";
pub(crate) const TOKEN_LIMIT: &str = "XML token before parser allocation";
pub(crate) const WINDOW_TEXT_LIMIT: &str = "window text";

impl OfficeError {
    /// One sentence for the viewer's status line, naming the kind of document and
    /// the reason without internal terms.
    pub fn message(&self, format: OfficeFormat) -> String {
        sentence(self, Some(format))
    }

    /// The internal reason, for Copy details and diagnostics.
    pub fn details(&self) -> String {
        match self {
            Self::Cancelled => "Office preview cancelled".into(),
            Self::Encrypted => "Encrypted Office package".into(),
            Self::Unsupported(reason) => format!("Unsupported Office content: {reason}"),
            Self::Malformed(reason) => format!("Malformed Office package: {reason}"),
            Self::LimitExceeded(reason) => format!("Office preview limit exceeded: {reason}"),
            Self::InvalidTarget => "Invalid Office preview target".into(),
            Self::Source(error) => format!("Office source read failed: {error}"),
        }
    }
}

fn sentence(error: &OfficeError, format: Option<OfficeFormat>) -> String {
    let (subject, kind) = match format {
        None => ("This file", "Office"),
        Some(OfficeFormat::Docx) => ("This document", "Word (.docx)"),
        Some(OfficeFormat::Xlsx) => ("This workbook", "Excel (.xlsx)"),
        Some(OfficeFormat::Pptx) => ("This presentation", "PowerPoint (.pptx)"),
        Some(OfficeFormat::Odt) => ("This document", "OpenDocument text (.odt)"),
        Some(OfficeFormat::Ods) => ("This spreadsheet", "OpenDocument spreadsheet (.ods)"),
        Some(OfficeFormat::Odp) => ("This presentation", "OpenDocument presentation (.odp)"),
    };
    match error {
        OfficeError::Cancelled => "The Office preview was cancelled.".into(),
        OfficeError::Encrypted => {
            format!("{subject} is password-protected, so the preview cannot open it.")
        }
        OfficeError::Unsupported(reason) => format!("{subject} {reason}."),
        OfficeError::Malformed(_) => format!(
            "{subject} is damaged or is not a valid {kind} file, so the preview cannot show it."
        ),
        OfficeError::LimitExceeded(limit) => {
            let reason = match *limit {
                CONTAINER_LIMIT => "is larger than 256 MiB, more than the preview opens",
                PART_COUNT_LIMIT => "has more than 10,000 parts, more than the preview opens",
                WORK_LIMIT => "needs more than 256 MiB of unpacked data for this page",
                TOKEN_LIMIT => "contains one text or markup item larger than 64 KiB",
                WINDOW_TEXT_LIMIT | "ODS window text" | "ODS row text" | "HTML resource" => {
                    "has more text on this page than the preview shows at once"
                }
                _ => "is larger or more complex than the preview's limits allow",
            };
            format!("{subject} {reason}.")
        }
        OfficeError::InvalidTarget => "The requested preview page does not exist.".into(),
        OfficeError::Source(error) => format!("{subject} could not be read: {error}"),
    }
}

impl fmt::Display for OfficeError {
    /// The status-line sentence without a document kind ("This file …").
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&sentence(self, None))
    }
}
impl std::error::Error for OfficeError {}
impl From<io::Error> for OfficeError {
    fn from(error: io::Error) -> Self {
        if error.get_ref().is_some_and(|inner| inner.is::<Self>()) {
            if let Some(inner) = error.into_inner()
                && let Ok(error) = inner.downcast::<Self>()
            {
                return *error;
            }
            return Self::Malformed("lost internal error");
        }
        Self::Source(error)
    }
}
impl From<zip::result::ZipError> for OfficeError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(error) => error.into(),
            _ => Self::Malformed("invalid ZIP structure or member"),
        }
    }
}
