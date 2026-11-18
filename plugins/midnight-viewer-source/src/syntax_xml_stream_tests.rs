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
        .expect("bounded XML streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 32, "unbounded ambiguous XML suffix");
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

#[test]
fn xml_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for text in [
        "<?xml version='1.0'?><文:根 e\u{301}:名=\"čeština\" n='a>b'/>tail",
        "<!-- a -- b ---><![CDATA[a ] b ]]]>text]]>&amp; &#123; &#x1f600; &invalid!",
        "<!DOCTYPE root [<!-- ] > --><!ENTITY x ']> literal'>]><root/>",
        "<root enormous_attribute_name='x'> &arbitrary_name; &#x; &; </root>",
        "<!- unfinished &not-an-entity;",
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
fn xml_stream_consumes_giant_names_entities_and_comments_without_buffers() {
    for prefix in ["<root ", "&", "<!--"] {
        let mut state = State::default();
        let mut styles = Styles::new(0..0, false);
        assert_eq!(
            stream(prefix, &mut state, &mut styles, false, || false),
            Some(prefix.len())
        );
        let chunk = "a".repeat(65_536);
        for _ in 0..20 {
            assert_eq!(
                stream(&chunk, &mut state, &mut styles, false, || false),
                Some(chunk.len())
            );
        }
        assert!(styles.finish().0.is_empty());
    }
    assert!(std::mem::size_of::<State>() <= 40);
}

#[test]
fn xml_stream_cancellation_and_depth_exhaustion_do_not_commit() {
    let mut state = State::default();
    let before = state;
    let calls = Cell::new(0);
    assert!(
        stream(
            &format!("<!--{}", "文".repeat(20_000)),
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
            &format!("<!DOCTYPE root {}", "[".repeat(17)),
            &mut state,
            &mut Styles::new(0..0, false),
            true,
            || false
        )
        .is_none()
    );
    assert_eq!(state, before);
}
