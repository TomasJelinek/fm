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
        .expect("bounded YAML streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 32, "unbounded ambiguous YAML suffix");
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

#[test]
fn yaml_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for text in [
        "name: midnight\nurl: https://host/path#fragment\nflow: {answer: 42, enabled: true}\n",
        "'quoted key'   : 'can''t \\'\n\"čeština\": \"a\\\"b\"\n",
        "literal: |2+ # header\n\n  true: # still string\n  文\nnext: false # comment\n",
        "folded: >-\n    content\n\n  dedent: 3\nlist: [1, 0x2a, -2.5e+3, .INF, ~]\n",
        "invalid: |22 nope\nquoted: 'first\nsecond': value\nspaces inside key  : value  \n",
        "mapping: plain scalar across many words # note\nblank: |\n \t\nnext: end",
        "  |2+\n    standalone literal\n  dedent: value\n",
        "plain: true # comment\r\nheader: |2+ # comment\r\n\r\n  body\r\nnext: 1\r\n",
        "normal: value # a\rb\r\r\n",
    ] {
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
fn yaml_stream_consumes_giant_keys_quotes_header_tails_and_block_indent() {
    for prefix in ["", "'", "key: |", "key: |\n"] {
        let mut state = State::default();
        let mut styles = Styles::new(0..0, false);
        assert_eq!(
            stream(prefix, &mut state, &mut styles, false, || false),
            Some(prefix.len())
        );
        let chunk = if prefix.ends_with('|') || prefix.ends_with('\n') {
            " ".repeat(65_536)
        } else {
            "a".repeat(65_536)
        };
        for _ in 0..20 {
            assert_eq!(
                stream(&chunk, &mut state, &mut styles, false, || false),
                Some(chunk.len())
            );
        }
        assert!(state.pending_style());
        assert!(styles.finish().0.is_empty());
    }
    assert!(std::mem::size_of::<State>() <= 64);
}

#[test]
fn yaml_stream_cancellation_is_transactional_within_a_giant_plain_scalar() {
    let mut state = State::default();
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
}

#[test]
fn yaml_comments_exclude_crlf_tails_and_block_bodies_include_them() {
    let text = "# first\r\r\nkey: |2+ # header\r\n  body\r\nnext: false\r\n";
    let mut styles = Styles::new(0..text.len(), false);
    scan(text, &mut State::default(), &mut styles, || false).unwrap();
    let spans = styles.finish().0;
    for comment in ["# first", "# header"] {
        let start = text.find(comment).unwrap();
        assert!(spans.iter().any(|span| span.role == STYLE_COMMENT
            && span.start as usize == start
            && span.end as usize == start + comment.len()));
    }
    let start = text.find("  body\r\n").unwrap();
    assert!(spans.iter().any(|span| span.role == STYLE_STRING
        && span.start as usize <= start
        && span.end as usize >= start + "  body\r\n".len()));
}
