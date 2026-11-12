use midnight_plugin_sdk::api::viewer::*;
use midnight_viewer_source::syntax::{self, State};

fn role(text: &str, spans: &[Span], needle: &str, expected: u16) {
    let start = text.find(needle).expect(needle);
    let end = start + needle.len();
    assert!(
        spans.iter().any(|span| span.role == expected
            && span.start as usize <= start
            && span.end as usize >= end),
        "{needle:?} expected role {expected}: {spans:?}"
    );
}
fn xml() -> midnight_viewer_source::syntax::Language {
    syntax::detect("document.xml", "").expect("XML requires an actual lexical profile")
}
#[test]
fn xml_and_project_extensions_route_to_one_profile_without_claiming_html_or_svg() {
    let language = xml();
    let extensions = include_str!("../plugin.toml")
        .lines()
        .find(|line| line.starts_with("extensions ="))
        .unwrap();
    for extension in [
        "xml", "xsd", "xsl", "xslt", "xaml", "csproj", "fsproj", "vbproj", "vcxproj", "props",
        "targets", "resx",
    ] {
        assert_eq!(
            syntax::detect(&format!("sample.{}", extension.to_uppercase()), ""),
            Some(language)
        );
        assert!(
            extensions.contains(&format!("\"{extension}\"")),
            "{extension} needs default routing"
        );
    }
    for extension in ["html", "htm", "svg"] {
        assert_eq!(syntax::detect(&format!("sample.{extension}"), ""), None);
        assert!(!extensions.contains(&format!("\"{extension}\"")));
    }
}
#[test]
fn xml_tags_attributes_literals_comments_cdata_and_processing_instructions_have_real_styles() {
    let text = "<?xml version=\"1.0\"?>\n<文:根 xml:lang=\"čeština\" n='42'>if true &amp; &#x1F600;<![CDATA[<literal>&x;]]><!-- note --></文:根>";
    let spans = syntax::highlight(text, xml(), &mut State::default(), || false).unwrap();
    for (needle, expected) in [
        ("xml", STYLE_KEYWORD),
        ("version", STYLE_TYPE),
        ("\"1.0\"", STYLE_STRING),
        ("文:根", STYLE_KEYWORD),
        ("xml:lang", STYLE_TYPE),
        ("\"čeština\"", STYLE_STRING),
        ("'42'", STYLE_STRING),
        ("&amp;", STYLE_OPERATOR),
        ("&#x1F600;", STYLE_NUMBER),
        ("<![CDATA[<literal>&x;]]>", STYLE_STRING),
        ("<!-- note -->", STYLE_COMMENT),
    ] {
        role(text, &spans, needle, expected);
    }
    let plain = text.find("if true").unwrap();
    assert!(
        !spans
            .iter()
            .any(|span| (span.start as usize..span.end as usize).contains(&plain)),
        "XML mixed content must not inherit programming-language keywords"
    );
}
#[test]
fn xml_multiline_lexical_context_recovers_after_each_delimited_construct() {
    for (first, second, carried, expected) in [
        (
            "<!-- first\r\n",
            "still --> <root/>",
            "still -->",
            STYLE_COMMENT,
        ),
        (
            "<root value=\"first\n",
            "second\">text</root>",
            "second\"",
            STYLE_STRING,
        ),
        (
            "<![CDATA[first\n",
            "<literal>]]><root/>",
            "<literal>]]>",
            STYLE_STRING,
        ),
        (
            "<?target\n",
            "option='a>b'?>\n<root/>",
            "option",
            STYLE_TYPE,
        ),
        (
            "<!DOCTYPE root [\n<!ENTITY x 'a > b'>\n",
            "]><root/>",
            "]>",
            STYLE_KEYWORD,
        ),
    ] {
        let mut state = State::default();
        syntax::highlight(first, xml(), &mut state, || false).unwrap();
        let spans = syntax::highlight(second, xml(), &mut state, || false).unwrap();
        role(second, &spans, carried, expected);
        role(second, &spans, "root", STYLE_KEYWORD);
    }
}
#[test]
fn xml_attribute_backslash_is_literal_and_unicode_names_keep_combining_marks() {
    let text = "<e\u{301}:文 a='back\\' other=\"quote'\"/>tail";
    let spans = syntax::highlight(text, xml(), &mut State::default(), || false).unwrap();
    role(text, &spans, "e\u{301}:文", STYLE_KEYWORD);
    role(text, &spans, "'back\\'", STYLE_STRING);
    role(text, &spans, "other", STYLE_TYPE);
    role(text, &spans, "\"quote'\"", STYLE_STRING);
}
#[test]
fn xml_limits_and_cancellation_preserve_the_last_valid_checkpoint() {
    let mut state = State::default();
    syntax::highlight("<!-- initial\n", xml(), &mut state, || false).unwrap();
    let before = state;
    let calls = std::cell::Cell::new(0);
    assert!(
        syntax::highlight(&"文".repeat(20_000), xml(), &mut state, || {
            calls.set(calls.get() + 1);
            calls.get() > 8
        })
        .is_none()
    );
    assert_eq!(state, before);
    assert!(syntax::highlight(&"x".repeat(65_537), xml(), &mut state, || false).is_none());
    assert_eq!(state, before);
    let mut state = State::default();
    let before = state;
    assert!(syntax::highlight(&"<a b='x'/>".repeat(2000), xml(), &mut state, || false).is_none());
    assert_eq!(state, before);
}
#[test]
fn xml_declarations_are_inert_and_bounded_without_confusing_nested_quotes_or_comments() {
    let text = "<!DOCTYPE root [<!-- ] > -->\n<!ENTITY x SYSTEM 'file:///missing.xml'>\n<!ENTITY y ']> value'>]><root/>";
    let spans = syntax::highlight(text, xml(), &mut State::default(), || false).unwrap();
    role(text, &spans, "<!-- ] > -->", STYLE_COMMENT);
    let last_root = text.rfind("root").unwrap();
    assert!(spans.iter().any(|span| span.role == STYLE_KEYWORD
        && span.start as usize == last_root
        && span.end as usize == last_root + 4));
    let mut state = State::default();
    let before = state;
    let excessive = format!("<!DOCTYPE root {}", "[".repeat(17));
    assert!(syntax::highlight(&excessive, xml(), &mut state, || false).is_none());
    assert_eq!(state, before);
}

#[test]
fn xml_delimiters_at_eof_finish_or_retain_the_exact_lexical_context() {
    for text in [
        "<root/>",
        "<?xml version='1.0'?>",
        "<!-- -->",
        "<![CDATA[]]>",
        "<!DOCTYPE root>",
        "<root key='value'>",
        "<root key=\"value\">",
    ] {
        let mut state = State::default();
        syntax::highlight(text, xml(), &mut state, || false).unwrap();
        assert_eq!(state, State::default(), "closed delimiter at EOF: {text}");
    }
    for (first, end, role_id) in [
        ("<root key='", "value'/>", STYLE_STRING),
        ("<!-- content", " -->", STYLE_COMMENT),
        ("<![CDATA[content", "]]>", STYLE_STRING),
    ] {
        let mut state = State::default();
        syntax::highlight(first, xml(), &mut state, || false).unwrap();
        assert_ne!(state, State::default());
        let spans = syntax::highlight(end, xml(), &mut state, || false).unwrap();
        assert_eq!(spans[0].role, role_id);
        assert_eq!(state, State::default());
    }
}
