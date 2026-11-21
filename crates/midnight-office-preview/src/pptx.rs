//! PresentationML catalog in relationship order, never ZIP-member order.
use crate::{
    OfficeCatalog, OfficeError, OfficeFormat, SlideDescriptor, SlideId,
    package::Package,
    xml::{Namespace, Token},
};
pub(crate) struct Pptx {
    pub catalog: OfficeCatalog,
    pub parts: Vec<String>,
}
impl Pptx {
    pub fn load(package: &mut Package) -> Result<Self, OfficeError> {
        let part = crate::workbook::office_part(package, crate::workbook::PRESENTATION)?;
        let (directory, file) = part.rsplit_once('/').unwrap_or(("", &part));
        let relation_path = if directory.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{directory}/_rels/{file}.rels")
        };
        let relations = crate::workbook::relationships(package, &relation_path, &part)?;
        let mut xml = package.xml(&part)?;
        let mut stack: Vec<(Namespace, String)> = Vec::new();
        let mut slides = Vec::new();
        let mut parts = Vec::new();
        let mut ids = std::collections::BTreeSet::new();
        let mut numeric_ids = std::collections::BTreeSet::new();
        let mut list_seen = false;
        loop {
            match xml.next()? {
                Token::Start(element) => {
                    if stack.is_empty() && !element.is(Namespace::Presentation, "presentation") {
                        return Err(OfficeError::Malformed("presentation root"));
                    }
                    if element.is(Namespace::Presentation, "sldIdLst") {
                        if list_seen || stack.len() != 1 {
                            return Err(OfficeError::Malformed("presentation slide list"));
                        }
                        list_seen = true;
                    }
                    if element.is(Namespace::Presentation, "sldId") {
                        if stack
                            .last()
                            .is_none_or(|(ns, n)| *ns != Namespace::Presentation || n != "sldIdLst")
                        {
                            return Err(OfficeError::Malformed("slide outside slide list"));
                        }
                        if slides.len() >= 4096 {
                            return Err(OfficeError::LimitExceeded("slide descriptors"));
                        }
                        let id = element
                            .attr_ns(Namespace::Relationships, "id")
                            .ok_or(OfficeError::Malformed("slide relationship ID"))?;
                        let number = element
                            .required("id")?
                            .parse::<u32>()
                            .map_err(|_| OfficeError::Malformed("slide numeric ID"))?;
                        if id.is_empty()
                            || id.len() > 1024
                            || number < 256
                            || !ids.insert(id.to_owned())
                            || !numeric_ids.insert(number)
                        {
                            return Err(OfficeError::Malformed("duplicate/invalid slide ID"));
                        }
                        let relationship = relations
                            .iter()
                            .find(|relation| relation.id == id)
                            .ok_or(OfficeError::Malformed("missing slide relationship"))?;
                        if !crate::workbook::kind(&relationship.kind, "slide") {
                            return Err(OfficeError::Malformed(
                                "slide list names a non-slide relationship",
                            ));
                        }
                        let target = relationship
                            .target
                            .as_ref()
                            .ok_or(OfficeError::Malformed("external slide"))?;
                        slides.push(SlideDescriptor {
                            id: SlideId(slides.len() as u16),
                            name: format!("Slide {}", slides.len() + 1),
                        });
                        parts.push(target.clone());
                    }
                    stack.push((element.ns, element.name));
                }
                Token::End(_, _) => {
                    stack.pop();
                }
                Token::Eof => break,
                Token::Text(_) => (),
            }
        }
        drop(xml);
        for part in &parts {
            if !package.contains(part) {
                return Err(OfficeError::Malformed("missing slide member"));
            }
        }
        Ok(Self {
            catalog: OfficeCatalog {
                format: OfficeFormat::Pptx,
                sheets: Vec::new(),
                document: None,
                slides,
            },
            parts,
        })
    }
}
pub(crate) fn window(
    package: &mut Package,
    part: &str,
    start: u32,
) -> Result<crate::document::Window, OfficeError> {
    crate::presentation::scan(
        package.xml(part)?,
        crate::presentation::Dialect::Drawing,
        start,
    )
}
