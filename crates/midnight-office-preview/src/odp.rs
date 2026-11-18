//! OpenDocument presentation catalog; pages remain in document order.
use crate::{
    OfficeCatalog, OfficeError, OfficeFormat, SlideDescriptor, SlideId,
    package::Package,
    xml::{Namespace, Token},
};
pub(crate) struct Odp {
    pub catalog: OfficeCatalog,
}
impl Odp {
    pub fn load(package: &mut Package) -> Result<Self, OfficeError> {
        crate::odt::load(package, "presentation")?;
        let mut xml = package.xml("content.xml")?;
        let mut stack: Vec<(Namespace, String)> = Vec::new();
        let mut slides = Vec::new();
        let mut body_seen = false;
        loop {
            match xml.next()? {
                Token::Start(element) => {
                    if stack.is_empty() && !element.is(Namespace::OdfOffice, "document-content") {
                        return Err(OfficeError::Unsupported("not an ODF contents document"));
                    }
                    if element.is(Namespace::OdfOffice, "presentation") {
                        if body_seen
                            || stack
                                .last()
                                .is_none_or(|(ns, n)| *ns != Namespace::OdfOffice || n != "body")
                        {
                            return Err(OfficeError::Malformed("ODP body"));
                        }
                        body_seen = true;
                    }
                    if element.is(Namespace::OdfDraw, "page")
                        && stack.last().is_some_and(|(ns, n)| {
                            *ns == Namespace::OdfOffice && n == "presentation"
                        })
                    {
                        if slides.len() >= 4096 {
                            return Err(OfficeError::LimitExceeded("slide descriptors"));
                        }
                        if element
                            .attr_ns(Namespace::OdfDraw, "name")
                            .is_some_and(|name| name.len() > 1024)
                        {
                            return Err(OfficeError::LimitExceeded("slide name"));
                        }
                        let name = format!("Slide {}", slides.len() + 1);
                        slides.push(SlideDescriptor {
                            id: SlideId(slides.len() as u16),
                            name,
                        });
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
        if !body_seen {
            return Err(OfficeError::Malformed("missing ODP body"));
        }
        Ok(Self {
            catalog: OfficeCatalog {
                format: OfficeFormat::Odp,
                sheets: Vec::new(),
                document: None,
                slides,
            },
        })
    }
}
pub(crate) fn window(
    package: &mut Package,
    slide: SlideId,
    start: u32,
) -> Result<crate::document::Window, OfficeError> {
    crate::presentation::scan(
        package.xml("content.xml")?,
        crate::presentation::Dialect::Odf(slide),
        start,
    )
}
