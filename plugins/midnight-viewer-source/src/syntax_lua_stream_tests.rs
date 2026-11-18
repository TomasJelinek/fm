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
        .expect("bounded Lua streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 32, "unbounded ambiguous Lua suffix");
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

#[test]
fn lua_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for text in [
        "#!/usr/bin/lua\nlocal café = 0x1.fp+2 -- ordinary\nreturn .5 .. 1..2\n",
        "--[==[ giant ]=] remains ]==] local x = [=[文 ]] ]=] return 7\n",
        "local x = \"a\\z  \n\t b\\\"c\"; local y = 'd\\\ntext'\n",
        "[==========[long]=========]still]==========] function f() end",
        "--[=====not a bracket\nreturn_value return truefalse false\n",
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
fn lua_long_bracket_counters_consume_giant_runs_without_a_token_buffer() {
    let mut state = State::default();
    let mut styles = Styles::new(0..0, false);
    assert_eq!(
        stream("[", &mut state, &mut styles, false, || false),
        Some(1)
    );
    let equals = "=".repeat(65_536);
    for _ in 0..20 {
        assert_eq!(
            stream(&equals, &mut state, &mut styles, false, || false),
            Some(equals.len())
        );
    }
    assert!(state.pending_style());
    assert_eq!(
        stream("[long text", &mut state, &mut styles, false, || false),
        Some(10)
    );
    assert!(!state.pending_style());
    assert!(styles.finish().0.is_empty());
    assert!(std::mem::size_of::<State>() <= 40);
}

#[test]
fn lua_stream_cancellation_is_transactional_and_bound_is_enforced() {
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
