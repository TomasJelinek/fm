use midnight_plugin_sdk::api::viewer::*;
use midnight_viewer_source::syntax::{self, State};

#[test]
fn batch_files_detect_case_insensitively_and_highlight_real_cmd_syntax() {
    for name in ["build.bat", "BUILD.BAT", "build.cmd", "BUILD.CmD"] {
        let language = syntax::detect(name, "@echo off\r\n")
            .unwrap_or_else(|| panic!("batch language missing for {name}"));
        let code =
            "@ECHO off\r\nREM comment\r\n:: comment\r\nIF exist \"C:\\tools\\\" SET count=42\r\n";
        let spans = syntax::highlight(code, language, &mut State::default(), || false).unwrap();
        for (token, role) in [
            ("ECHO", STYLE_KEYWORD),
            ("REM comment\r", STYLE_COMMENT),
            (":: comment\r", STYLE_COMMENT),
            ("IF", STYLE_KEYWORD),
            ("SET", STYLE_KEYWORD),
            ("\"C:\\tools\\\"", STYLE_STRING),
            ("42", STYLE_NUMBER),
        ] {
            assert!(
                spans.iter().any(|span| span.role == role
                    && &code[span.start as usize..span.end as usize] == token),
                "{name}: missing style {role} for {token:?}: {spans:?}"
            );
        }
    }
}

#[test]
fn batch_comment_words_inside_arguments_and_single_quotes_stay_plain() {
    let language = syntax::detect("x.bat", "").expect("batch syntax");
    let code =
        "echo reminder REM ordinary\necho 'unquoted\nIF exist file (\n  @rem nested comment\n)\n";
    let spans = syntax::highlight(code, language, &mut State::default(), || false).unwrap();
    assert!(!spans.iter().any(|span| span.role == STYLE_STRING));
    let comments: Vec<_> = spans
        .iter()
        .filter(|span| span.role == STYLE_COMMENT)
        .map(|span| &code[span.start as usize..span.end as usize])
        .collect();
    assert_eq!(comments, ["rem nested comment"]);
    assert!(
        spans.iter().any(|span| span.role == STYLE_KEYWORD
            && &code[span.start as usize..span.end as usize] == "IF")
    );
}
