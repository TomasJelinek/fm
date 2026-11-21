//! Bounded backend-only Office contents previews from captured positional bytes.
//! No formula evaluation, path opening, document links or unread-tail integrity claims.
#[cfg(test)]
mod compatibility_tests;
mod document;
mod docx;
#[cfg(test)]
mod format_tests;
#[cfg(test)]
mod guard_tests;
mod html;
mod index;
mod model;
mod odp;
mod ods;
mod odt;
mod package;
mod pptx;
mod preflight;
#[cfg(test)]
mod preflight_tests;
mod presentation;
mod sheet;
mod source;
mod workbook;
mod xml;

pub use midnight_core::Cancellation;
pub use model::{
    DocumentDescriptor, DocumentId, ImmutableHtml, OfficeCatalog, OfficeError, OfficeFormat,
    OfficeTarget, SheetDescriptor, SheetId, SlideDescriptor, SlideId,
};
pub use source::OfficeSource;
use std::sync::Arc;

enum FormatState {
    Xlsx(workbook::Workbook),
    Docx {
        part: String,
        catalog: OfficeCatalog,
    },
    Odt {
        catalog: OfficeCatalog,
    },
    Ods(ods::Ods),
    Pptx(pptx::Pptx),
    Odp(odp::Odp),
}
/// Synchronous service; construction, rendering and destruction belong on workers.
pub struct OfficeSession {
    package: package::Package,
    state: FormatState,
    generation: u64,
}
impl OfficeSession {
    /// Opens one captured package with fixed, non-increasable security limits.
    pub fn open(
        source: Arc<dyn OfficeSource>,
        format: OfficeFormat,
        cancel: &Cancellation,
    ) -> Result<Self, OfficeError> {
        if cancel.is_cancelled() {
            return Err(OfficeError::Cancelled);
        }
        let mut package = package::Package::open(source, cancel)?;
        let state = match format {
            OfficeFormat::Xlsx => FormatState::Xlsx(workbook::Workbook::load(&mut package)?),
            OfficeFormat::Docx => FormatState::Docx {
                part: docx::load(&mut package)?,
                catalog: document_catalog(format),
            },
            OfficeFormat::Odt => {
                odt::load(&mut package, "text")?;
                FormatState::Odt {
                    catalog: document_catalog(format),
                }
            }
            OfficeFormat::Ods => FormatState::Ods(ods::Ods::load(&mut package)?),
            OfficeFormat::Pptx => FormatState::Pptx(pptx::Pptx::load(&mut package)?),
            OfficeFormat::Odp => FormatState::Odp(odp::Odp::load(&mut package)?),
        };
        package.check()?;
        Ok(Self {
            package,
            state,
            generation: 0,
        })
    }
    /// Ordered bounded descriptors; IDs are local to this session.
    pub fn catalog(&self) -> &OfficeCatalog {
        match &self.state {
            FormatState::Xlsx(value) => &value.catalog,
            FormatState::Docx { catalog, .. } | FormatState::Odt { catalog } => catalog,
            FormatState::Ods(value) => &value.catalog,
            FormatState::Pptx(value) => &value.catalog,
            FormatState::Odp(value) => &value.catalog,
        }
    }
    /// Metadata-only selection: no reads, parsing, rendering or generation changes.
    /// Catalog is the fallback only when this family's catalog has no content target.
    pub fn initial_target(&self) -> OfficeTarget {
        let catalog = self.catalog();
        match catalog.format {
            OfficeFormat::Docx | OfficeFormat::Odt => {
                catalog
                    .document
                    .as_ref()
                    .map(|entry| OfficeTarget::DocumentWindow {
                        document: entry.id,
                        block: 0,
                    })
            }
            OfficeFormat::Xlsx | OfficeFormat::Ods => catalog
                .sheets
                .first()
                .map(|entry| OfficeTarget::SheetFirst { sheet: entry.id }),
            OfficeFormat::Pptx | OfficeFormat::Odp => {
                catalog
                    .slides
                    .first()
                    .map(|entry| OfficeTarget::SlideWindow {
                        slide: entry.id,
                        block: 0,
                    })
            }
        }
        .unwrap_or(OfficeTarget::Catalog)
    }
    pub fn parse_target(&self, path: &str) -> Result<OfficeTarget, OfficeError> {
        let target = OfficeTarget::parse(path)?;
        self.validate(target)?;
        Ok(target)
    }
    /// Generates one inert immutable page; errors never publish partial bytes.
    pub fn render(
        &mut self,
        target: OfficeTarget,
        cancel: &Cancellation,
    ) -> Result<ImmutableHtml, OfficeError> {
        self.validate(target)?;
        self.package.begin(cancel)?;
        let bytes = match (&self.state, target) {
            (_, OfficeTarget::Catalog) => html::catalog(self.catalog())?,
            (
                FormatState::Xlsx(workbook),
                OfficeTarget::SheetFirst { sheet } | OfficeTarget::SheetWindow { sheet, .. },
            ) => match workbook
                .parts
                .get(usize::from(sheet.0))
                .ok_or(OfficeError::InvalidTarget)?
            {
                crate::workbook::SheetPart::Cells(part) => {
                    let shared = workbook.shared_strings.as_deref();
                    let window = sheet::load(&mut self.package, part, shared, target)?;
                    html::sheet(&workbook.catalog, sheet, &window)?
                }
                crate::workbook::SheetPart::Unshown(reason) => {
                    html::unshown_sheet(&workbook.catalog, sheet, reason)?
                }
            },
            (
                FormatState::Ods(workbook),
                OfficeTarget::SheetFirst { sheet } | OfficeTarget::SheetWindow { sheet, .. },
            ) => {
                let window = ods::window(&mut self.package, target)?;
                html::sheet(&workbook.catalog, sheet, &window)?
            }
            (FormatState::Docx { part, catalog }, OfficeTarget::DocumentWindow { block, .. }) => {
                let window = docx::window(&mut self.package, part, block)?;
                document::render(catalog, target, &window)?
            }
            (FormatState::Odt { catalog }, OfficeTarget::DocumentWindow { block, .. }) => {
                let window = odt::window(&mut self.package, block)?;
                document::render(catalog, target, &window)?
            }
            (FormatState::Pptx(presentation), OfficeTarget::SlideWindow { slide, block }) => {
                let part = presentation
                    .parts
                    .get(usize::from(slide.0))
                    .ok_or(OfficeError::InvalidTarget)?;
                let window = pptx::window(&mut self.package, part, block)?;
                document::render(&presentation.catalog, target, &window)?
            }
            (FormatState::Odp(presentation), OfficeTarget::SlideWindow { slide, block }) => {
                let window = odp::window(&mut self.package, slide, block)?;
                document::render(&presentation.catalog, target, &window)?
            }
            _ => return Err(OfficeError::InvalidTarget),
        };
        self.package.check()?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(OfficeError::LimitExceeded("generation"))?;
        Ok(ImmutableHtml {
            bytes: Arc::from(bytes),
            target,
            generation: self.generation,
        })
    }
    fn validate(&self, target: OfficeTarget) -> Result<(), OfficeError> {
        target.validate()?;
        let catalog = self.catalog();
        let member = match (catalog.format, target) {
            (_, OfficeTarget::Catalog) => true,
            (
                OfficeFormat::Xlsx | OfficeFormat::Ods,
                OfficeTarget::SheetFirst { sheet } | OfficeTarget::SheetWindow { sheet, .. },
            ) => catalog
                .sheets
                .get(usize::from(sheet.0))
                .is_some_and(|entry| entry.id == sheet),
            (
                OfficeFormat::Docx | OfficeFormat::Odt,
                OfficeTarget::DocumentWindow { document, .. },
            ) => {
                document.0 == 0
                    && catalog
                        .document
                        .as_ref()
                        .is_some_and(|entry| entry.id == document)
            }
            (OfficeFormat::Pptx | OfficeFormat::Odp, OfficeTarget::SlideWindow { slide, .. }) => {
                catalog
                    .slides
                    .get(usize::from(slide.0))
                    .is_some_and(|entry| entry.id == slide)
            }
            _ => false,
        };
        if member {
            Ok(())
        } else {
            Err(OfficeError::InvalidTarget)
        }
    }
}
fn document_catalog(format: OfficeFormat) -> OfficeCatalog {
    OfficeCatalog {
        format,
        sheets: Vec::new(),
        slides: Vec::new(),
        document: Some(DocumentDescriptor {
            id: DocumentId(0),
            name: "Document".to_owned(),
        }),
    }
}
