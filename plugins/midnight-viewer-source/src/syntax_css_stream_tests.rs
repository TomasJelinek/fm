use super::*;
use crate::styles::Styles;
use std::cell::Cell;

fn run(text: &str, ends: impl IntoIterator<Item = usize>) -> (State, Vec<Span>) {
    let mut state = State::default();
    let mut styles = Styles::new(0..text.len(), false);
    let mut at = 0;
    for end in ends {
        styles.origin(at);
        let consumed = stream(
            &text[at..end],
            &mut state,
            &mut styles,
            end == text.len(),
            || false,
        )
        .expect("bounded CSS streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 32, "unbounded ambiguous CSS suffix");
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

fn role_of(text: &str, spans: &[Span], token: &str) -> u16 {
    let start = text
        .find(token)
        .unwrap_or_else(|| panic!("{token:?} in fixture"));
    let end = start + token.len();
    spans
        .iter()
        .find(|span| span.start as usize <= start && span.end as usize >= end)
        .map_or(STYLE_PLAIN, |span| span.role)
}

const FIXTURES: [&str; 8] = [
    "body {\n  color: #e0e0e0;\n  background: #101820;\n}\n",
    "@charset \"UTF-8\";\n@import url(\"theme.css\") screen;\n/* comment * / still */\n.a.b > #id:hover::before, [data-x~='y z' i] { margin: -0.5em auto 10px; }\n",
    "@media screen and (min-width: 600px) {\n  .card { width: calc(100% - 2rem) !important; }\n}\n@font-face { font-family: \"Ünï\"; src: url(fonts/a.woff2) format('woff2'); }\n",
    "@keyframes spin {\n  from { transform: rotate(0deg); }\n  50% { opacity: .5 }\n  to { transform: rotate(360deg); }\n}\n",
    ":root { --main-color: #ABCDEF80; --gap: 4px; }\n.x { color: var(--main-color); border: 1px solid #12345; }\n",
    ".parent { color: red; &:hover { color: blue; } .child > span { x: 1e3; } }\n@supports (display: grid) { @media print { a { b: c } } }\n",
    "a { content: \"un\\\"closed\nb { color: rgb(1 2 3 / 50%); background: url( data:image/png;base64,iVBOR\\) ) no-repeat }\n",
    "\\31 23 { x: y }\n.é\\\n { }\n#\u{1f600} { -webkit-x: --y; +.5 }\n} } ; @ ; !important",
];

#[test]
fn css_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for text in FIXTURES {
        let expected = run(text, [text.len()]);
        let boundaries: Vec<_> = text.char_indices().map(|(at, _)| at).skip(1).collect();
        for &split in &boundaries {
            assert_eq!(
                run(text, [split, text.len()]),
                expected,
                "split {split}: {text}"
            );
        }
        assert_eq!(
            run(text, boundaries.into_iter().chain([text.len()])),
            expected
        );
    }
}

#[test]
fn css_roles_follow_selector_declaration_and_at_rule_context() {
    let text = FIXTURES[0];
    let spans = run(text, [text.len()]).1;
    assert_eq!(role_of(text, &spans, "body"), STYLE_FUNCTION);
    assert_eq!(role_of(text, &spans, "color"), STYLE_TYPE);
    assert_eq!(role_of(text, &spans, "#e0e0e0"), STYLE_NUMBER);
    assert_eq!(role_of(text, &spans, "background"), STYLE_TYPE);
    assert_eq!(role_of(text, &spans, "#101820"), STYLE_NUMBER);

    let text = FIXTURES[1];
    let spans = run(text, [text.len()]).1;
    for (token, role) in [
        ("@charset", STYLE_KEYWORD),
        ("\"UTF-8\"", STYLE_STRING),
        ("@import", STYLE_KEYWORD),
        ("url", STYLE_FUNCTION),
        ("\"theme.css\"", STYLE_STRING),
        ("screen", STYLE_PLAIN),
        ("/* comment * / still */", STYLE_COMMENT),
        (".a.b", STYLE_FUNCTION),
        ("#id:hover::before", STYLE_FUNCTION),
        ("data-x", STYLE_FUNCTION),
        ("'y z'", STYLE_STRING),
        ("margin", STYLE_TYPE),
        ("-0.5em", STYLE_NUMBER),
        ("auto", STYLE_PLAIN),
        ("10px", STYLE_NUMBER),
    ] {
        assert_eq!(role_of(text, &spans, token), role, "{token}: {spans:?}");
    }

    let text = FIXTURES[2];
    let spans = run(text, [text.len()]).1;
    for (token, role) in [
        ("@media", STYLE_KEYWORD),
        ("min-width", STYLE_TYPE),
        ("600px", STYLE_NUMBER),
        (".card", STYLE_FUNCTION),
        ("width", STYLE_TYPE),
        ("calc", STYLE_FUNCTION),
        ("100%", STYLE_NUMBER),
        ("2rem", STYLE_NUMBER),
        ("!important", STYLE_KEYWORD),
        ("@font-face", STYLE_KEYWORD),
        ("font-family", STYLE_TYPE),
        ("\"Ünï\"", STYLE_STRING),
        ("fonts/a.woff2", STYLE_STRING),
        ("format", STYLE_FUNCTION),
        ("'woff2'", STYLE_STRING),
    ] {
        assert_eq!(role_of(text, &spans, token), role, "{token}: {spans:?}");
    }

    let text = FIXTURES[3];
    let spans = run(text, [text.len()]).1;
    for (token, role) in [
        ("@keyframes", STYLE_KEYWORD),
        ("from", STYLE_FUNCTION),
        ("transform", STYLE_TYPE),
        ("rotate", STYLE_FUNCTION),
        ("0deg", STYLE_NUMBER),
        ("50%", STYLE_NUMBER),
        (".5", STYLE_NUMBER),
        ("to {", STYLE_FUNCTION),
    ] {
        let token = token.trim_end_matches(" {");
        assert_eq!(role_of(text, &spans, token), role, "{token}: {spans:?}");
    }

    let text = FIXTURES[4];
    let spans = run(text, [text.len()]).1;
    assert_eq!(role_of(text, &spans, ":root"), STYLE_FUNCTION);
    assert_eq!(role_of(text, &spans, "--main-color"), STYLE_TYPE);
    assert_eq!(role_of(text, &spans, "#ABCDEF80"), STYLE_NUMBER);
    assert_eq!(role_of(text, &spans, "var"), STYLE_FUNCTION);
    let reference = text.find("(--main-color)").unwrap() + 1;
    assert!(spans.iter().any(|span| span.role == STYLE_TYPE
        && span.start as usize == reference
        && span.end as usize == reference + "--main-color".len()));
    // Five hexadecimal digits are not a colour.
    assert_eq!(role_of(text, &spans, "#12345"), STYLE_PLAIN);

    let text = FIXTURES[5];
    let spans = run(text, [text.len()]).1;
    for (token, role) in [
        (".parent", STYLE_FUNCTION),
        ("&:hover", STYLE_FUNCTION),
        (".child", STYLE_FUNCTION),
        ("span", STYLE_FUNCTION),
        ("1e3", STYLE_NUMBER),
        ("@supports", STYLE_KEYWORD),
        ("display", STYLE_TYPE),
        ("@media print", STYLE_KEYWORD),
    ] {
        let token = token.trim_end_matches(" print");
        assert_eq!(role_of(text, &spans, token), role, "{token}: {spans:?}");
    }
    // `@media` inside `@supports` still holds rules: `a` is a selector, `b` a property.
    let nested = text.find("a { b: c }").unwrap();
    assert!(spans.iter().any(|span| span.role == STYLE_FUNCTION
        && span.start as usize == nested
        && span.end as usize == nested + 1));
    assert!(spans.iter().any(|span| span.role == STYLE_TYPE
        && span.start as usize == nested + 4
        && span.end as usize == nested + 5));
}

#[test]
fn css_strings_end_at_a_line_break_and_unquoted_urls_end_at_their_parenthesis() {
    let text = FIXTURES[6];
    let spans = run(text, [text.len()]).1;
    let open = text.find("\"un").unwrap();
    let line = text.find('\n').unwrap();
    assert!(spans.iter().any(|span| span.role == STYLE_STRING
        && span.start as usize == open
        && span.end as usize == line));
    // The next line is ordinary CSS again: `b` opens a rule.
    assert_eq!(role_of(text, &spans, "color"), STYLE_TYPE);
    assert_eq!(role_of(text, &spans, "rgb"), STYLE_FUNCTION);
    assert_eq!(
        role_of(text, &spans, "data:image/png;base64,iVBOR\\)"),
        STYLE_STRING
    );
    assert_eq!(role_of(text, &spans, "no-repeat"), STYLE_PLAIN);
}

#[test]
fn css_stream_consumes_giant_names_values_and_comments_without_a_token_buffer() {
    for (prefix, chunk) in [
        ("a { ", "x".repeat(65_536)),
        ("a { b: ", "x".repeat(65_536)),
        ("@media ", "x".repeat(65_536)),
        ("/*", "*".repeat(65_536)),
        ("a { b: url(", "y".repeat(65_536)),
    ] {
        let mut state = State::default();
        let mut styles = Styles::new(0..0, false);
        assert_eq!(
            stream(prefix, &mut state, &mut styles, false, || false),
            Some(prefix.len())
        );
        for _ in 0..20 {
            assert_eq!(
                stream(&chunk, &mut state, &mut styles, false, || false),
                Some(chunk.len()),
                "{prefix}"
            );
        }
    }
    let mut state = State::default();
    let mut styles = Styles::new(0..0, false);
    stream("a { ", &mut state, &mut styles, false, || false);
    stream(&"x".repeat(65_536), &mut state, &mut styles, false, || {
        false
    });
    assert!(state.pending_style());
    assert_eq!(state.pending_span(), Some((65_536, 0)));
    assert_eq!(
        stream(" :", &mut state, &mut styles, false, || false),
        Some(2)
    );
    assert!(!state.pending_style());
    assert!(std::mem::size_of::<State>() <= 48);
}

#[test]
fn css_block_kinds_saturate_beyond_the_tracked_depth() {
    let open = "@media x {".repeat(70);
    let text = format!("{open} a {{ b: c }}{} d {{ e: f }}", "}".repeat(70));
    let (state, spans) = run(&text, [text.len()]);
    assert_eq!(state.depth, 0);
    assert_eq!(state.rules, 0);
    // Beyond 64 tracked levels blocks hold declarations, where a name before `{`
    // is still a nested selector.
    let a = text.find(" a {").unwrap() + 1;
    assert!(
        spans
            .iter()
            .any(|span| span.start as usize == a && span.role == STYLE_FUNCTION)
    );
    let d = text.find(" d {").unwrap() + 1;
    assert!(
        spans
            .iter()
            .any(|span| span.start as usize == d && span.role == STYLE_FUNCTION)
    );
    let e = text.find("e: f").unwrap();
    assert!(
        spans
            .iter()
            .any(|span| span.start as usize == e && span.role == STYLE_TYPE)
    );
}

#[test]
fn css_stream_cancellation_is_transactional_and_bound_is_enforced() {
    let mut state = State::default();
    stream(
        "a { b",
        &mut state,
        &mut Styles::new(0..0, false),
        false,
        || false,
    );
    let before = state;
    let calls = Cell::new(0);
    assert!(
        stream(
            &"x".repeat(65_536),
            &mut state,
            &mut Styles::new(0..0, false),
            false,
            || {
                calls.set(calls.get() + 1);
                calls.get() > 4
            }
        )
        .is_none()
    );
    assert_eq!(state, before);
    assert!(
        stream(
            &"x".repeat(65_537),
            &mut state,
            &mut Styles::new(0..0, false),
            false,
            || false
        )
        .is_none()
    );
    assert_eq!(state, before);
}

/// Deterministic xorshift soup of CSS punctuation, names, digits and Unicode.
fn soup(seed: u64, length: usize) -> String {
    const PIECES: [&str; 40] = [
        "a", "b", "-", "--", "_", "0", "7", ".", "#", ":", "::", ";", "{", "}", "(", ")", "[", "]",
        "=", "\"", "'", "\\", "/", "*", "/*", "*/", "@", "@media", "!", "%", "url(", " ", "\n",
        "\t", "é", "😀", ",", ">", "+", "&",
    ];
    let mut state = seed | 1;
    let mut text = String::new();
    while text.len() < length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push_str(PIECES[(state % PIECES.len() as u64) as usize]);
    }
    text
}

#[test]
fn css_random_soup_never_panics_and_is_split_invariant() {
    for seed in 0..64 {
        let text = soup(seed, 600);
        let expected = run(&text, [text.len()]);
        let boundaries: Vec<_> = text.char_indices().map(|(at, _)| at).collect();
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        for _ in 0..24 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut ends: Vec<usize> = (0..4)
                .map(|shift| boundaries[((state >> (shift * 16)) as usize) % boundaries.len()])
                .collect();
            ends.sort_unstable();
            ends.dedup();
            ends.retain(|end| *end != 0);
            ends.push(text.len());
            assert_eq!(run(&text, ends.clone()), expected, "{ends:?}: {text:?}");
        }
    }
}

#[test]
fn an_escaped_crlf_continues_a_string_like_an_escaped_lf() {
    for text in [
        "a { content: \"x\\\r\ny\"; color: red }\r\n",
        "a { content: \"x\\\ny\"; color: red }\n",
    ] {
        let expected = run(text, [text.len()]);
        assert_eq!(role_of(text, &expected.1, "y\""), STYLE_STRING, "{text:?}");
        assert_ne!(
            role_of(text, &expected.1, "color"),
            STYLE_STRING,
            "{text:?}"
        );
        for split in 1..text.len() {
            assert_eq!(
                run(text, [split, text.len()]),
                expected,
                "split {split}: {text:?}"
            );
        }
    }
    // A lone escaped CR is one escaped line break; the quote after it closes the string.
    let text = "a { content: \"x\\\r\"; color: red }\n";
    let (_, spans) = run(text, [text.len()]);
    assert_ne!(role_of(text, &spans, "color"), STYLE_STRING);
}
