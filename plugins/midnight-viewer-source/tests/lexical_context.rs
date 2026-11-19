//! Delimiters that are not string literals in a language must not open string
//! state that the viewer would carry across lines and checkpoints.
use midnight_plugin_sdk::api::viewer::*;
use midnight_viewer_source::syntax::{self, Language, State};
use std::time::{Duration, Instant};

/// Styled fragments of `code`, lexed one line at a time as viewer pages are.
fn lex(name: &str, code: &str) -> Vec<(String, u16)> {
    let language = syntax::detect(name, "").expect(name);
    let mut state = State::default();
    let mut styled = Vec::new();
    for line in code.split_inclusive('\n') {
        let lexed = syntax::highlight(line, language, &mut state, || false).expect(name);
        assert!(!lexed.truncated);
        styled.extend(
            lexed
                .spans
                .iter()
                .map(|s| (line[s.start as usize..s.end as usize].to_owned(), s.role)),
        );
    }
    styled
}

fn has(styled: &[(String, u16)], text: &str, role: u16) -> bool {
    styled.iter().any(|(t, r)| t == text && *r == role)
}

fn strings(styled: &[(String, u16)]) -> Vec<&str> {
    styled
        .iter()
        .filter(|(_, role)| *role == STYLE_STRING)
        .map(|(text, _)| text.as_str())
        .collect()
}

#[test]
fn rust_lifetimes_and_labels_are_not_character_literals() {
    let code = "struct Parser<'a> {\n    input: &'a str,\n}\nfn f<'b>(x: &'b u8) -> char {\n    'outer: loop { break 'outer; }\n    let c = 'x';\n    let e = '\\'';\n    let b = b'q';\n}\nlet done = 1; // comment\n";
    let styled = lex("x.rs", code);
    assert_eq!(strings(&styled), ["'x'", "'\\''", "'q'"], "{styled:?}");
    assert!(has(&styled, "let", STYLE_KEYWORD));
    assert!(has(&styled, "loop", STYLE_KEYWORD));
    assert!(has(&styled, "// comment", STYLE_COMMENT), "{styled:?}");
}

#[test]
fn single_line_literals_end_at_an_unescaped_newline() {
    for (name, code, keyword) in [
        ("x.c", "char *s = \"unterminated;\nint x = 1;\n", "int"),
        ("x.cpp", "char c = 'q;\nint x = 1;\n", "int"),
        ("x.java", "String s = \"open;\nint x = 1;\n", "int"),
        ("x.cs", "string s = \"open;\nint x = 1;\n", "int"),
        ("x.go", "s := \"open\nvar x = 1\n", "var"),
        ("x.js", "const s = 'open;\nlet x = 1;\n", "let"),
        ("x.ts", "const s = \"open;\nlet x = 1;\n", "let"),
        ("x.py", "s = 'open\nif x: pass\n", "if"),
        ("x.kt", "val s = \"open\nwhile (x) {}\n", "while"),
        ("x.swift", "let s = \"open\nwhile x {}\n", "while"),
        ("x.bat", "echo \"open\nif exist x echo y\n", "if"),
    ] {
        let styled = lex(name, code);
        assert!(
            has(&styled, keyword, STYLE_KEYWORD),
            "{name}: second line stays code: {styled:?}"
        );
    }
    // Escaped newlines continue C-like and Python literals.
    let styled = lex("x.c", "char *s = \"one \\\ntwo\";\nint x = 1;\n");
    assert!(has(&styled, "two\"", STYLE_STRING), "{styled:?}");
    assert!(has(&styled, "int", STYLE_KEYWORD));
}

#[test]
fn multiline_literals_still_span_lines() {
    for (name, code, inside, keyword) in [
        (
            "x.rs",
            "let s = \"one\ntwo\";\nwhile x {}\n",
            "two\"",
            "while",
        ),
        (
            "x.js",
            "let s = `one\ntwo`;\nwhile (x) {}\n",
            "two`",
            "while",
        ),
        ("x.go", "s := `one\ntwo`\nvar x = 1\n", "two`", "var"),
        ("x.py", "s = '''one\ntwo'''\nif x: pass\n", "two'''", "if"),
        (
            "x.toml",
            "s = '''one\ntwo'''\nok = true\n",
            "two'''",
            "true",
        ),
        (
            "x.java",
            "String s = \"\"\"\ntwo\"\"\";\nint x = 1;\n",
            "two\"\"\"",
            "int",
        ),
        (
            "x.sh",
            "echo 'one\ntwo'\nif true; then :; fi\n",
            "two'",
            "if",
        ),
        (
            "x.cs",
            "var p = @\"one\ntwo\";\nint x = 1;\n",
            "two\"",
            "int",
        ),
    ] {
        let styled = lex(name, code);
        assert!(has(&styled, inside, STYLE_STRING), "{name}: {styled:?}");
        assert!(has(&styled, keyword, STYLE_KEYWORD), "{name}: {styled:?}");
    }
}

#[test]
fn apostrophes_in_json_comments_and_yaml_plain_scalars_open_no_quotes() {
    let styled = lex(
        "tsconfig.json",
        "{\n  // don't do this\n  \"strict\": true\n}\n",
    );
    assert_eq!(strings(&styled), ["\"strict\""], "{styled:?}");
    assert!(has(&styled, "true", STYLE_KEYWORD));

    // The YAML lexer colours a plain value as one string span, so the apostrophe
    // stays inside "Don't fail" and opens no quote: the next keys and the quoted
    // scalars are lexed on their own.
    let code = "name: Don't fail\nquoted: 'it''s'\nlist: ['a', \"b\"]\nenabled: true\n";
    let styled = lex("x.yaml", code);
    assert_eq!(
        strings(&styled),
        ["Don't fail", "'it''s'", "'a'", "\"b\""],
        "{styled:?}"
    );
    for key in ["quoted", "list", "enabled"] {
        assert!(has(&styled, key, STYLE_TYPE), "{key}: {styled:?}");
    }
    assert!(has(&styled, "true", STYLE_KEYWORD));
}

#[test]
fn literal_backslashes_do_not_escape_raw_or_single_quoted_delimiters() {
    for (name, code, literal, keyword) in [
        (
            "x.toml",
            "path = 'C:\\dir\\'\nok = true\n",
            "'C:\\dir\\'",
            "true",
        ),
        (
            "x.sh",
            "cp 'C:\\dir\\' x\nif true; then :; fi\n",
            "'C:\\dir\\'",
            "if",
        ),
        (
            "x.sql",
            "SELECT 'C:\\dir\\' FROM t;\nDELETE FROM t;\n",
            "'C:\\dir\\'",
            "DELETE",
        ),
        (
            "x.yaml",
            "path: 'C:\\dir\\'\nok: true\n",
            "'C:\\dir\\'",
            "true",
        ),
        (
            "x.cs",
            "var p = @\"C:\\dir\\\";\nint q = 1;\n",
            "\"C:\\dir\\\"",
            "int",
        ),
        (
            "x.go",
            "p := `C:\\dir\\`\nvar q = 1\n",
            "`C:\\dir\\`",
            "var",
        ),
    ] {
        let styled = lex(name, code);
        assert!(has(&styled, literal, STYLE_STRING), "{name}: {styled:?}");
        assert!(has(&styled, keyword, STYLE_KEYWORD), "{name}: {styled:?}");
    }
}

#[test]
fn long_batch_lines_with_non_leading_comment_markers_lex_in_linear_time() {
    for line in [
        format!("echo {}\n", ":".repeat(40_000)),
        format!("echo {}\n", "rem ".repeat(15_000)),
    ] {
        let started = Instant::now();
        let spans = syntax::highlight(&line, Language::Batch, &mut State::default(), || false);
        let elapsed = started.elapsed();
        assert!(spans.is_some());
        assert!(
            elapsed < Duration::from_secs(2),
            "{} byte Batch line took {elapsed:?}",
            line.len()
        );
    }
    // Comment detection still follows command separators exactly.
    let styled = lex(
        "x.cmd",
        "echo a & rem one\necho b :: two\n@ @rem three\n(::four\n",
    );
    let comments: Vec<_> = styled
        .iter()
        .filter(|(_, role)| *role == STYLE_COMMENT)
        .map(|(text, _)| text.as_str())
        .collect();
    assert_eq!(comments, ["rem one", "::four"], "{styled:?}");
}
