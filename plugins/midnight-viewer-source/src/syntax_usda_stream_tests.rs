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
        .expect("bounded USDA streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 32, "unbounded ambiguous USDA suffix");
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

#[test]
fn usda_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for text in [
        "#usda 1.0\ndef Xform \"č😀\" { uniform token t = 'x'; float f = -.5e+2 }\n",
        "/* comment * / */ over \"Owned\" { rel x = </Owned/Child.attr>; asset a = @one@ } // tail",
        "string x = '''a\n# inert </path> @value@\n b'''; double f = -inf; double n = nan\n",
        "asset x = @@@a\\@@@b@@@; asset y = @@@two@@at@@@@@; def Scope \"Done\" {}",
        "def /* keep type slot */ Xform \"Owned\" { point3f[] p = [(1., .2, -3E-4)] }\n",
        "string a = 'bad\ndef Scope \"Next\" {}\nasset b = @@@bad\nover \"Next\" {}\n",
        "floatSuffix primvars:float names::def 文float 1e 1e+ 1e- 1e2 -infx\n",
    ] {
        let expected = run(text, [text.len()]);
        let boundaries: Vec<_> = text.char_indices().map(|(at, _)| at).skip(1).collect();
        for &split in &boundaries {
            assert_eq!(run(text, [split, text.len()]), expected, "split {split}");
        }
        assert_eq!(
            run(text, boundaries.into_iter().chain([text.len()])),
            expected
        );
    }
}

#[test]
fn usda_giant_tokens_and_delimiters_use_finite_streaming_state() {
    for (open, body, close, role) in [
        ("'''", "owned body ", "'''", STYLE_STRING),
        ("@@@", "owned@path/", "@@@", STYLE_STRING),
        ("/*", "owned * text ", "*/", STYLE_COMMENT),
        ("", "identifier", " ", STYLE_PLAIN),
        ("", "1234567890", " ", STYLE_NUMBER),
    ] {
        let text = format!(
            "{open}{}{close} def Xform \"Done\" {{}}",
            body.repeat(15000)
        );
        let ends = (32000..text.len()).step_by(32000).chain([text.len()]);
        let (state, spans) = run(&text, ends);
        assert_eq!(state, State::default());
        assert!(std::mem::size_of::<State>() <= 32);
        if role != STYLE_PLAIN {
            assert!(
                spans
                    .iter()
                    .any(|s| s.start == 0 && s.end as usize >= text.len() - 25 && s.role == role)
            );
        }
        let def = text.rfind("def").unwrap();
        assert!(
            spans
                .iter()
                .any(|s| s.start as usize == def && s.role == STYLE_KEYWORD)
        );
    }
}

#[test]
fn usda_stream_cancellation_is_transactional_and_turn_bound_is_enforced() {
    let mut state = State::default();
    stream(
        "'''open\n",
        &mut state,
        &mut Styles::new(0..0, false),
        false,
        || false,
    )
    .unwrap();
    let before = state;
    let continuation = format!("'''{}", " ".repeat(65533));
    let mut completed = before;
    let calls = Cell::new(0);
    assert_eq!(
        stream(
            &continuation,
            &mut completed,
            &mut Styles::new(0..0, false),
            false,
            || {
                calls.set(calls.get() + 1);
                false
            }
        ),
        Some(continuation.len())
    );
    assert_eq!(completed, State::default());
    assert_ne!(completed, before);
    let completed_calls = calls.get();
    assert!(completed_calls > 2);
    // The closing delimiter precedes both the middle and final cancellation polls.
    for cancel_at in [completed_calls / 2, completed_calls] {
        calls.set(0);
        assert!(
            stream(
                &continuation,
                &mut state,
                &mut Styles::new(0..0, false),
                false,
                || {
                    calls.set(calls.get() + 1);
                    calls.get() == cancel_at
                }
            )
            .is_none()
        );
        assert_eq!(calls.get(), cancel_at);
        assert_eq!(state, before);
    }
    assert!(
        stream(
            &"x".repeat(65537),
            &mut state,
            &mut Styles::new(0..0, false),
            false,
            || false
        )
        .is_none()
    );
    assert_eq!(state, before);
}

#[test]
fn usda_style_collection_overflow_keeps_multiline_checkpoint_progress() {
    let text = format!("{}'''open\n", "float x = 1;\n".repeat(2000));
    let language = crate::syntax::detect("owned.usda", "").expect("USDA profile");
    let mut state = crate::syntax::State::default();
    let lexed = crate::syntax::highlight(&text, language, &mut state, || false).unwrap();
    assert!(lexed.truncated);
    assert_eq!(lexed.spans.len(), MAX_SPANS);
    let next = "close''' def Xform \"Done\" {}";
    let lexed = crate::syntax::highlight(next, language, &mut state, || false).unwrap();
    assert_eq!(lexed.spans[0].start, 0);
    assert_eq!(lexed.spans[0].end, 8);
    assert_eq!(lexed.spans[0].role, STYLE_STRING);
    let at = next.find("def").unwrap() as u32;
    assert!(
        lexed
            .spans
            .iter()
            .any(|s| s.start == at && s.role == STYLE_KEYWORD)
    );
}
