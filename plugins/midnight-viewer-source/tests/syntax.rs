use midnight_plugin_sdk::api::viewer::*;
use midnight_viewer_source::syntax::{self, Language, State};

#[test]
fn every_declared_language_recognizes_keywords_strings_comments_and_numbers() {
    let cases = [
        (
            "x.rs",
            Language::Rust,
            "let x = 42; // comment\n\"hello\"",
            "let",
            "// comment",
        ),
        (
            "x.c",
            Language::C,
            "int x = 42; // comment\n\"hello\"",
            "int",
            "// comment",
        ),
        (
            "x.cpp",
            Language::Cpp,
            "class X {}; // comment\n42; \"hello\"",
            "class",
            "// comment",
        ),
        (
            "x.js",
            Language::JavaScript,
            "const x = 42; // comment\n\"hello\"",
            "const",
            "// comment",
        ),
        (
            "x.ts",
            Language::TypeScript,
            "interface X {} // comment\n42; \"hello\"",
            "interface",
            "// comment",
        ),
        (
            "x.py",
            Language::Python,
            "def f(): # comment\n return 42, \"hello\"",
            "def",
            "# comment",
        ),
        (
            "x.sh",
            Language::Shell,
            "if true; # comment\necho 42 \"hello\"",
            "if",
            "# comment",
        ),
        (
            "x.json",
            Language::Json,
            "{\"hello\": 42, \"ok\": true}",
            "true",
            "",
        ),
        (
            "x.toml",
            Language::Toml,
            "hello = true # comment\nvalue = 42\ns = \"hello\"",
            "true",
            "# comment",
        ),
    ];
    for (name, language, code, keyword, comment) in cases {
        assert_eq!(syntax::detect(name, ""), Some(language), "{name}");
        let spans = syntax::highlight(code, language, &mut State::default(), || false).unwrap();
        for (token, role) in [
            (keyword, STYLE_KEYWORD),
            ("42", STYLE_NUMBER),
            ("\"hello\"", STYLE_STRING),
            (comment, STYLE_COMMENT),
        ] {
            if token.is_empty() {
                continue;
            }
            assert!(
                spans.iter().any(|span| span.role == role
                    && &code[span.start as usize..span.end as usize] == token),
                "{name} {token} {spans:?}"
            );
        }
    }
}

#[test]
fn shebang_and_unknown_fallback() {
    assert_eq!(
        syntax::detect("script", "#!/usr/bin/env python3\n"),
        Some(Language::Python)
    );
    assert_eq!(
        syntax::detect("script", "#!/bin/bash\n"),
        Some(Language::Shell)
    );
    assert_eq!(
        syntax::detect("script", "#!/usr/bin/env node\n"),
        Some(Language::JavaScript)
    );
    assert_eq!(syntax::detect("unknown.xyz", "not a shebang python"), None);
}

#[test]
fn nested_rust_comments_and_multiline_strings_keep_context_between_pages() {
    let mut state = State::default();
    syntax::highlight(
        "/* outer\n/* nested */\n",
        Language::Rust,
        &mut state,
        || false,
    )
    .unwrap();
    let spans = syntax::highlight(
        "still comment */ let x;",
        Language::Rust,
        &mut state,
        || false,
    )
    .unwrap();
    assert_eq!(spans[0].role, STYLE_COMMENT);
    assert_eq!(spans[0].end, 16);
    assert!(spans.iter().any(|span| span.role == STYLE_KEYWORD));
    let mut state = State::default();
    syntax::highlight("x = \"\"\"multi\n", Language::Python, &mut state, || false).unwrap();
    let spans = syntax::highlight(
        "line\"\"\"\nreturn 42",
        Language::Python,
        &mut state,
        || false,
    )
    .unwrap();
    assert_eq!(spans[0].role, STYLE_STRING);
    assert!(spans.iter().any(|span| span.role == STYLE_KEYWORD));
}

#[test]
fn lexical_resource_limits_and_cancellation_return_plain_fallback() {
    assert!(
        syntax::highlight("let x = 1", Language::Rust, &mut State::default(), || false).is_some()
    );
    assert!(
        syntax::highlight("let x = 1", Language::Rust, &mut State::default(), || true).is_none()
    );
    assert!(
        syntax::highlight(
            &"x ".repeat(100_000),
            Language::Rust,
            &mut State::default(),
            || false
        )
        .is_none()
    );
    assert!(
        syntax::highlight(
            &"/*".repeat(100),
            Language::Rust,
            &mut State::default(),
            || false
        )
        .is_none()
    );
}

#[test]
fn additional_common_lexical_profiles_have_real_token_coverage() {
    for (name, code, keyword, comment) in [
        (
            "Test.java",
            "class Test { int n = 42; } // comment\n\"hello\"",
            "class",
            "// comment",
        ),
        (
            "Test.cs",
            "namespace Test { int n = 42; } // comment\n\"hello\"",
            "namespace",
            "// comment",
        ),
        (
            "test.go",
            "func main() { n := 42; } // comment\n\"hello\"",
            "func",
            "// comment",
        ),
        (
            "test.swift",
            "func main() { let n = 42; } // comment\n\"hello\"",
            "func",
            "// comment",
        ),
        (
            "test.kt",
            "fun main() { val n = 42; } // comment\n\"hello\"",
            "fun",
            "// comment",
        ),
        (
            "test.rb",
            "def main; n = 42; end # comment\n\"hello\"",
            "def",
            "# comment",
        ),
        (
            "test.sql",
            "SELECT 42 FROM table; -- comment\n\"hello\"",
            "SELECT",
            "-- comment",
        ),
        (
            "test.yaml",
            "enabled: true # comment\nvalue: 42\nname: \"hello\"",
            "true",
            "# comment",
        ),
    ] {
        let language = syntax::detect(name, "").expect(name);
        let spans = syntax::highlight(code, language, &mut State::default(), || false).unwrap();
        for (text, role) in [
            (keyword, STYLE_KEYWORD),
            (comment, STYLE_COMMENT),
            ("42", STYLE_NUMBER),
            ("\"hello\"", STYLE_STRING),
        ] {
            assert!(
                spans.iter().any(|span| span.role == role
                    && &code[span.start as usize..span.end as usize] == text),
                "{name}: {text}"
            );
        }
    }
}
