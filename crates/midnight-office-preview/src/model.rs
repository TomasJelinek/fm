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

#[derive(Debug)]
pub enum OfficeError {
    Cancelled,
    Encrypted,
    Unsupported(&'static str),
    Malformed(&'static str),
    LimitExceeded(&'static str),
    InvalidTarget,
    Source(io::Error),
}
impl fmt::Display for OfficeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Office preview cancelled"),
            Self::Encrypted => f.write_str("Encrypted Office package is unsupported"),
            Self::Unsupported(reason) => write!(f, "Unsupported Office content: {reason}"),
            Self::Malformed(reason) => write!(f, "Malformed Office package: {reason}"),
            Self::LimitExceeded(reason) => write!(f, "Office preview limit exceeded: {reason}"),
            Self::InvalidTarget => f.write_str("Invalid Office preview target"),
            Self::Source(error) => write!(f, "Office source read failed: {error}"),
        }
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
