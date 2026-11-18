use super::*;

#[test]
fn unicode_runs_check_cancellation_before_emitting_the_whole_run() {
    use std::cell::Cell;
    for (language, prefix) in [
        (Language::Cpp, "\""),
        (Language::Cpp, "R\"("),
        (Language::Rust, "r#\""),
        (Language::Cpp, "/* "),
    ] {
        let text = format!("{prefix}{}", "é".repeat(20_000));
        let calls = Cell::new(0);
        let mut state = State::default();
        let mut styles = Styles::new(0..text.len(), false);
        let result = stream(&text, language, &mut state, &mut styles, false, || {
            calls.set(calls.get() + 1);
            calls.get() >= 4
        });
        assert!(result.is_none(), "{language:?} {prefix}");
        assert_eq!(state, State::default(), "cancel must not commit");
        assert!(
            styles.finish().0.iter().all(|s| s.end <= 128),
            "{language:?} {prefix}: entire Unicode run emitted before cancellation"
        );
    }
}

fn incremental(text: &str, language: Language, chunks: Vec<&str>) -> (Vec<Span>, State) {
    let mut state = State::default();
    let mut styles = Styles::new(0..text.len(), false);
    let mut pending = String::new();
    let mut offset = 0;
    for chunk in chunks {
        pending.push_str(chunk);
        styles.origin(offset);
        let consumed = stream(&pending, language, &mut state, &mut styles, false, || false)
            .expect("bounded stream");
        assert!(pending.is_char_boundary(consumed));
        assert!(consumed <= pending.len());
        pending.drain(..consumed);
        offset += consumed;
        assert!(
            pending.len() <= 128,
            "bounded ambiguous suffix, no token buffer"
        );
    }
    styles.origin(offset);
    assert_eq!(
        stream(&pending, language, &mut state, &mut styles, true, || false),
        Some(pending.len())
    );
    (styles.finish().0, state)
}

#[test]
fn generic_stream_matches_complete_scan_at_every_scalar_boundary() {
    let cases = [
        (
            Language::Cpp,
            "/* é */ int x = 123.45e+6; // comment\nreturn R\"tag(a \" /*\nraw)tag\";\n",
        ),
        (
            Language::Rust,
            "/* outer /* inner */ é */ let s=r###\"raw\n\"## no\n\"###;\n",
        ),
        (
            Language::Python,
            "def f(): # comment\n value = \"\"\"hello\nworld\"\"\"\n return 42\n",
        ),
        (Language::Shell, "if true; # comment\necho 'hello'\n"),
        (
            Language::Sql,
            "SELECT 42 FROM table; -- comment\nWHERE id = 1\n",
        ),
        (
            Language::Batch,
            "@echo off\r\nREM comment\r\necho OK & rem second\r\n",
        ),
    ];
    for (language, text) in cases {
        let mut expected_state = State::default();
        let expected = highlight(text, language, &mut expected_state, || false).unwrap();
        for split in (0..=text.len()).filter(|at| text.is_char_boundary(*at)) {
            let (actual, _) = incremental(text, language, vec![&text[..split], &text[split..]]);
            assert_eq!(actual, expected, "{language:?} split {split}");
        }
        let chunks = text
            .char_indices()
            .map(|(at, ch)| &text[at..at + ch.len_utf8()])
            .collect();
        assert_eq!(
            incremental(text, language, chunks).0,
            expected,
            "{language:?} scalar turns"
        );
    }
}

#[test]
fn generic_giant_identifier_and_line_comment_do_not_buffer_the_line() {
    for (text, role) in [
        (format!("{}\nreturn 7;\n", "a".repeat(131072)), STYLE_PLAIN),
        (
            format!("//{}\nreturn 7;\n", "a".repeat(131072)),
            STYLE_COMMENT,
        ),
    ] {
        let mut state = State::default();
        let mut offset = 0;
        let mut styles = Styles::new(0..text.len(), false);
        let mut pending = String::new();
        for chunk in text.as_bytes().chunks(4096) {
            pending.push_str(std::str::from_utf8(chunk).unwrap());
            styles.origin(offset);
            let consumed = stream(
                &pending,
                Language::Cpp,
                &mut state,
                &mut styles,
                false,
                || false,
            )
            .unwrap();
            offset += consumed;
            pending.drain(..consumed);
            assert!(pending.len() <= 128);
        }
        styles.origin(offset);
        assert_eq!(
            stream(
                &pending,
                Language::Cpp,
                &mut state,
                &mut styles,
                true,
                || false
            ),
            Some(pending.len())
        );
        let spans = styles.finish().0;
        let at = text.find("return").unwrap() as u32;
        assert!(
            spans
                .iter()
                .any(|s| s.start == at && s.role == STYLE_KEYWORD)
        );
        if role == STYLE_COMMENT {
            assert_eq!(spans[0].role, STYLE_COMMENT);
            assert_eq!(spans[0].end, 131074);
        } else {
            assert!(!spans.iter().any(|s| s.start < at));
        }
    }
}
