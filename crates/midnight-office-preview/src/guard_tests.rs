use crate::{
    Cancellation, OfficeError,
    package::relationship_target,
    source::{Decoded, MAX_WORK, Work},
    xml::{Token, Xml},
};
use std::io::{Cursor, Read};

#[test]
fn xml_token_guard_rejects_oversized_text_attributes_comments_and_cdata() {
    for xml in [
        format!("<root>{}</root>", "x".repeat(65_537)),
        format!("<root attribute='{}'/>", "x".repeat(65_537)),
        format!("<root><!--{}--></root>", "x".repeat(65_537)),
        format!("<root><![CDATA[{}]]></root>", "x".repeat(65_537)),
    ] {
        let mut parser = Xml::new(Cursor::new(xml));
        loop {
            match parser.next() {
                Err(OfficeError::LimitExceeded(_)) => break,
                Err(error) => panic!("unexpected error: {error}"),
                Ok(Token::Eof) => panic!("oversized token accepted"),
                Ok(_) => (),
            }
        }
    }
}

#[test]
fn decoded_budget_is_reserved_before_output_and_cannot_deliver_over_limit() {
    let work = Work::new(&Cancellation::default());
    assert_eq!(
        work.reserve((MAX_WORK - 3) as usize).unwrap(),
        (MAX_WORK - 3) as usize
    );
    let mut decoded = Decoded {
        inner: Cursor::new(b"abcdef"),
        work,
    };
    let mut bytes = [0; 8];
    assert_eq!(decoded.read(&mut bytes).unwrap(), 3);
    assert_eq!(&bytes[..3], b"abc");
    let error = OfficeError::from(decoded.read(&mut bytes).unwrap_err());
    assert!(matches!(error, OfficeError::LimitExceeded(_)));
}

#[test]
fn package_root_and_relative_relationships_have_same_captured_target() {
    assert_eq!(
        relationship_target("xl/workbook.xml", "/xl/worksheets/sheet1.xml").unwrap(),
        "xl/worksheets/sheet1.xml"
    );
    assert_eq!(
        relationship_target("xl/workbook.xml", "worksheets/sheet1.xml").unwrap(),
        "xl/worksheets/sheet1.xml"
    );
    assert_eq!(
        relationship_target("xl/workbook.xml", "../xl/worksheets/sheet1.xml").unwrap(),
        "xl/worksheets/sheet1.xml"
    );
    for target in [
        "/../outside",
        "//authority/path",
        "/%2e%2e/outside",
        "/",
        "",
    ] {
        assert!(relationship_target("xl/workbook.xml", target).is_err());
    }
}

#[test]
fn xml_depth_and_qualified_name_storage_are_bounded() {
    for xml in [
        format!("{}{}", "<a>".repeat(129), "</a>".repeat(129)),
        format!("<{} />", "n".repeat(257)),
    ] {
        let mut parser = Xml::new(Cursor::new(xml));
        loop {
            match parser.next() {
                Err(OfficeError::LimitExceeded(_)) => break,
                Err(error) => panic!("unexpected error: {error}"),
                Ok(Token::Eof) => panic!("XML allocation guard missing"),
                Ok(_) => (),
            }
        }
    }
}
