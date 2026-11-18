use super::{Guard, TOKEN, Token, Xml};
use crate::OfficeError;
use std::io::{self, BufRead, Cursor, Read};

struct Short<R> {
    input: R,
    chunk: usize,
}
impl<R: Read> Read for Short<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(self.chunk);
        self.input.read(&mut output[..length])
    }
}

#[test]
fn pi_and_declaration_internal_tags_cannot_grow_parser_event_buffer() {
    for prefix in ["<?p>", "<?xml version='1.0' >"] {
        for suffix in ["?>", ""] {
            for chunk in [1, 3, 8192] {
                let input = format!("{prefix}{}{suffix}", "<x/>".repeat(TOKEN / 4 + 1));
                let mut parser = Xml::new(Short {
                    input: Cursor::new(input),
                    chunk,
                });
                assert!(matches!(parser.next(), Err(OfficeError::LimitExceeded(_))));
                assert!(
                    parser.buffer.is_empty(),
                    "unsafe partial PI reached quick-xml"
                );
                assert_eq!(parser.buffer.capacity(), TOKEN, "parser event storage grew");
            }
        }
    }
}

#[test]
fn pi_guard_caps_whole_lexical_token_before_exposure_or_growth() {
    let input = format!("<?p>{}?>", "<x/>".repeat(TOKEN / 4 + 1));
    let mut guard = Guard::new(Short {
        input: Cursor::new(input),
        chunk: 3,
    });
    assert!(guard.fill_buf().is_err());
    assert_eq!(guard.position, 0);
    assert_eq!(guard.token.len(), TOKEN);
    assert_eq!(guard.token.capacity(), TOKEN);
}

#[test]
fn declaration_termination_is_exact_across_single_byte_reads() {
    let input = "<?xml version='1.0' encoding='UTF-8'?><root/>";
    let mut parser = Xml::new(Short {
        input: Cursor::new(input),
        chunk: 1,
    });
    assert!(matches!(parser.next(), Ok(Token::Start(element)) if element.name == "root"));
    assert!(matches!(parser.next(), Ok(Token::End(_, name)) if name == "root"));
    assert!(matches!(parser.next(), Ok(Token::Eof)));
}

fn qualified_name_error(error: &io::Error) -> bool {
    matches!(error.get_ref().and_then(|error| error.downcast_ref::<OfficeError>()),
        Some(OfficeError::LimitExceeded(reason)) if reason.contains("name"))
}

#[test]
fn qualified_slashed_names_are_bounded_before_parser_exposure() {
    for input in [
        format!("<a/{}:local>", "x".repeat(512)),
        format!("<a/{}:local/>", "x".repeat(512)),
        format!("</a/{}:local>", "x".repeat(512)),
        format!("</a {}>", "x".repeat(512)),
    ] {
        for chunk in [1, 3, 8192] {
            let mut guard = Guard::new(Short {
                input: Cursor::new(&input),
                chunk,
            });
            let error = match guard.fill_buf() {
                Err(error) => error,
                Ok(_) => panic!("a complete over-limit QName reached the parser"),
            };
            assert!(qualified_name_error(&error), "{error}");
            assert_eq!(guard.position, 0);
            assert_eq!(guard.token.capacity(), TOKEN);
        }
    }
}

#[test]
fn qualified_over_limit_element_names_never_reach_quick_xml_storage() {
    for input in [
        format!("<{} />", "a".repeat(257)),
        format!("<a/{}:local/>", "x".repeat(512)),
        format!("</a/{}:local>", "x".repeat(512)),
    ] {
        let mut parser = Xml::new(Cursor::new(input));
        assert!(matches!(parser.next(), Err(OfficeError::LimitExceeded(_))));
        assert!(
            parser.buffer.is_empty(),
            "over-limit name reached quick-xml"
        );
        assert_eq!(parser.buffer.capacity(), TOKEN);
    }
}

#[test]
fn qualified_attribute_and_namespace_names_are_bounded_before_parser_exposure() {
    for name in [
        "a".repeat(257),
        format!("a/{}:local", "x".repeat(512)),
        format!("xmlns:{}", "p".repeat(251)),
    ] {
        for quote in ['\'', '"'] {
            let input = format!("<root {name} = {quote}value/with&gt;markup{quote}/>");
            let mut guard = Guard::new(Short {
                input: Cursor::new(input),
                chunk: 1,
            });
            let error = match guard.fill_buf() {
                Err(error) => error,
                Ok(_) => panic!("an over-limit attribute QName reached the parser"),
            };
            assert!(qualified_name_error(&error), "{error}");
            assert_eq!(guard.position, 0);
        }
    }
}

#[test]
fn qualified_exact_byte_boundary_preserves_empty_tags_and_unicode_namespaces() {
    let name = "é".repeat(128);
    assert_eq!(name.len(), 256);
    for input in [
        format!("<{name}/>"),
        format!("<{name}></{name} \t>"),
        format!("<{name} />"),
    ] {
        let mut parser = Xml::new(Short {
            input: Cursor::new(input),
            chunk: 1,
        });
        assert!(matches!(parser.next(), Ok(Token::Start(element)) if element.name == name));
        assert!(matches!(parser.next(), Ok(Token::End(_, end)) if end == name));
        assert!(matches!(parser.next(), Ok(Token::Eof)));
    }
    let local = "文".repeat(84);
    let qualified = format!("前:{local}");
    assert_eq!(qualified.len(), 256);
    let input = format!("<{qualified} xmlns:前='urn:example' {qualified}=\"text/with&gt;slash\"/>");
    let mut parser = Xml::new(Cursor::new(input));
    assert!(matches!(parser.next(), Ok(Token::Start(element)) if element.name == local));
    assert!(matches!(parser.next(), Ok(Token::End(_, end)) if end == local));
    assert!(matches!(parser.next(), Ok(Token::Eof)));
}

#[test]
fn qualified_attribute_values_do_not_count_as_names_or_tag_delimiters() {
    let value = "value/with>markup:".repeat(100);
    let input = format!("<root attribute = '{value}' other=\"/>&gt;\"></root>");
    let mut parser = Xml::new(Cursor::new(input));
    assert!(
        matches!(parser.next(), Ok(Token::Start(element)) if element.attr("attribute") == Some(value.as_str()) && element.attr("other") == Some("/>>"))
    );
    assert!(matches!(parser.next(), Ok(Token::End(_, end)) if end == "root"));
    assert!(matches!(parser.next(), Ok(Token::Eof)));
}

#[test]
fn qualified_internal_slashes_are_rejected_without_treating_them_as_delimiters() {
    for input in ["<a/b>", "<a/b/>", "</a/b>", "<root a/b='value'/>"] {
        let mut guard = Guard::new(Cursor::new(input));
        assert!(
            guard.fill_buf().is_err(),
            "accepted malformed QName {input}"
        );
        assert_eq!(guard.position, 0);
    }
}
