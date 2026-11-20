#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::{
    midnight_plugin_entry_v1 as entry,
    syntax::{self, State},
};
use support::Host;

fn styled(text: &str, spans: &[Span], token: &str, role: u16) {
    let start = text.find(token).expect("fixture token") as u32;
    let end = start + token.len() as u32;
    assert!(
        spans
            .iter()
            .any(|s| s.start <= start && s.end >= end && s.role == role),
        "{token:?} needs role {role}: {spans:?}"
    );
}

/// No span touches the first `token`: all of it keeps the text colour.
fn unstyled(text: &str, spans: &[Span], token: &str) {
    let start = text.find(token).expect("fixture token") as u32;
    let end = start + token.len() as u32;
    assert!(
        spans.iter().all(|s| s.end <= start || s.start >= end),
        "{token:?} needs the plain role: {spans:?}"
    );
}

fn lex(name: &str, text: &str, state: &mut State) -> Vec<Span> {
    let language = syntax::detect(name, "").expect("recognized language");
    syntax::highlight(text, language, state, || false)
        .expect("bounded lexical output")
        .spans
}

#[test]
fn lua_has_real_tokens_and_routing_in_the_public_plugin() {
    let text = "local value = 0x2a -- ordinary comment\nreturn 'hello'\n";
    let mut host = Host::new(text.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 7,
            length: Some(text.len() as u64),
            mode: MODE_SOURCE,
            display_name: "SCRIPT.LUA".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 8192,
            max_lines: 512,
        },
    );
    let events = host.finish();
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("source page")
            .payload,
    )
    .unwrap();
    for (token, role) in [
        ("local", STYLE_KEYWORD),
        ("0x2a", STYLE_NUMBER),
        ("-- ordinary comment", STYLE_COMMENT),
        ("return", STYLE_KEYWORD),
        ("'hello'", STYLE_STRING),
    ] {
        styled(&page.text, &page.spans, token, role);
    }
    assert_eq!(
        syntax::detect("script", "#!/usr/bin/env lua5.4\n"),
        syntax::detect("a.lua", "")
    );
}

/// Open `text` as `name` in source mode through the public plugin and return its first page.
fn source_page(name: &str, text: &str) -> TextPage {
    let mut host = Host::new(text.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 7,
            length: Some(text.len() as u64),
            mode: MODE_SOURCE,
            display_name: name.into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 8192,
            max_lines: 512,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("source page")
            .payload,
    )
    .unwrap()
}

#[test]
fn css_selectors_properties_values_and_at_rules_are_coloured_in_the_public_plugin() {
    let text = "@media (min-width: 600px) {\n  body.dark > #main:hover {\n    color: #e0e0e0;\n    margin: 0 auto -1.5em;\n    background: url(img/bg.png), rgb(1 2 3) !important; /* note */\n    font-family: \"Noto Sans\";\n  }\n}\n";
    let page = source_page("style.css", text);
    assert_eq!(page.text, text);
    for (token, role) in [
        ("@media", STYLE_KEYWORD),
        ("min-width", STYLE_TYPE),
        ("600px", STYLE_NUMBER),
        ("body.dark", STYLE_FUNCTION),
        ("#main:hover", STYLE_FUNCTION),
        ("color", STYLE_TYPE),
        ("#e0e0e0", STYLE_NUMBER),
        ("margin", STYLE_TYPE),
        ("-1.5em", STYLE_NUMBER),
        ("url", STYLE_FUNCTION),
        ("img/bg.png", STYLE_STRING),
        ("rgb", STYLE_FUNCTION),
        ("!important", STYLE_KEYWORD),
        ("/* note */", STYLE_COMMENT),
        ("\"Noto Sans\"", STYLE_STRING),
    ] {
        styled(&page.text, &page.spans, token, role);
    }
    page.validate().unwrap();
}

#[test]
fn makefile_targets_variables_recipes_and_directives_are_coloured_in_the_public_plugin() {
    let text = "# Build\nCC ?= cc\nOBJ := $(patsubst %.c,%.o,$(wildcard *.c))\n.PHONY: all\nall: app\n\napp: $(OBJ)\n\t@$(CC) -o $@ $^\nifdef DEBUG\nexport CFLAGS += -g\nendif\n";
    for name in [
        "Makefile",
        "makefile",
        "GNUmakefile",
        "rules.mk",
        "rules.MAK",
    ] {
        let page = source_page(name, text);
        assert_eq!(page.text, text, "{name}");
        for (token, role) in [
            ("# Build", STYLE_COMMENT),
            ("CC", STYLE_TYPE),
            // An assignment operator and its value.
            (" ?= cc", STYLE_PLAIN),
            ("OBJ", STYLE_TYPE),
            ("patsubst", STYLE_FUNCTION),
            // A function's arguments inside its reference.
            (" %.c,%.o,", STYLE_PLAIN),
            ("wildcard", STYLE_FUNCTION),
            ("$(OBJ)", STYLE_TYPE),
            (".PHONY", STYLE_KEYWORD),
            // Rule separators and prerequisites (the targets are checked below).
            (": all", STYLE_PLAIN),
            (": app", STYLE_PLAIN),
            ("-o ", STYLE_STRING),
            ("$@", STYLE_TYPE),
            ("ifdef", STYLE_KEYWORD),
            ("DEBUG", STYLE_TYPE),
            ("export", STYLE_KEYWORD),
            ("CFLAGS", STYLE_TYPE),
            (" += -g", STYLE_PLAIN),
            ("endif", STYLE_KEYWORD),
        ] {
            if role == STYLE_PLAIN {
                unstyled(&page.text, &page.spans, token);
            } else {
                styled(&page.text, &page.spans, token, role);
            }
        }
        for target in ["all:", "app:"] {
            let start = page.text.find(&format!("\n{target}")).unwrap() as u32 + 1;
            assert!(
                page.spans.iter().any(|span| span.role == STYLE_FUNCTION
                    && span.start == start
                    && span.end == start + 3),
                "{name}: target {target}: {:?}",
                page.spans
            );
        }
        page.validate().unwrap();
    }
}

#[test]
fn css_and_makefile_names_and_extensions_route_to_their_profiles() {
    let css = syntax::detect("style.css", "").expect("CSS profile");
    let make = syntax::detect("Makefile", "").expect("Makefile profile");
    assert_ne!(css, make);
    assert_eq!(syntax::detect("THEME.CSS", ""), Some(css));
    for name in [
        "makefile",
        "GNUmakefile",
        "GNUMAKEFILE",
        "x.mk",
        "x.MAK",
        "dir.makefile",
    ] {
        assert_eq!(syntax::detect(name, ""), Some(make), "{name}");
    }
    for name in [
        "notmakefile",
        "Makefile.am",
        "Makefile.in",
        "x.scss",
        "x.mkd",
    ] {
        assert_eq!(syntax::detect(name, ""), None, "{name}");
    }
    let extensions = include_str!("../plugin.toml")
        .lines()
        .find(|line| line.starts_with("extensions ="))
        .unwrap();
    for extension in ["css", "mk", "mak", "makefile", "gnumakefile"] {
        assert_eq!(
            extensions.matches(&format!("\"{extension}\"")).count(),
            1,
            "{extension} needs default routing"
        );
    }
}

/// The next page from `anchor` of a session that `host` already opened.
fn page_at(host: &mut Host, anchor: u64) -> TextPage {
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 8192,
            max_lines: 512,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("source page")
            .payload,
    )
    .unwrap()
}

#[test]
fn csv_and_tsv_fields_take_the_colour_of_their_column_in_the_public_plugin() {
    // The first lines of the project's own docs/third-party-inventory.csv.
    let text = r#""Package","Version","Direct","License","WindowsFeatures","LinuxFeatures","WindowsGraph","LinuxGraph","PackagedRootNotices","Repository","PinnedSource"
"ab_glyph","0.2.32","False","Apache-2.0","","default,gvar-alloc,std,variable-fonts","False","True","LICENSE","https://github.com/alexheretic/ab-glyph","https://docs.rs/crate/ab_glyph/0.2.32/source/"
"#;
    let page = source_page("third-party-inventory.csv", text);
    assert_eq!(page.text, text);
    for (token, role) in [
        ("Version", STYLE_STRING),
        ("Direct", STYLE_NUMBER),
        ("License", STYLE_FUNCTION),
        ("WindowsFeatures", STYLE_TYPE),
        ("WindowsGraph", STYLE_STRING),
        ("0.2.32", STYLE_STRING),
        ("False", STYLE_NUMBER),
        ("Apache-2.0", STYLE_FUNCTION),
        // After the commas inside the sixth field: the eighth column.
        ("True", STYLE_NUMBER),
        ("LICENSE", STYLE_FUNCTION),
        ("https://github.com/alexheretic/ab-glyph", STYLE_TYPE),
    ] {
        styled(&page.text, &page.spans, token, role);
    }
    // Quotes stay plain, and so do delimiters between them; the first column keeps the
    // text colour.
    assert!(
        page.spans
            .iter()
            .all(|span| !page.text[span.start as usize..span.end as usize].contains('"'))
    );
    for first in ["Package", "ab_glyph"] {
        let start = page.text.find(first).unwrap() as u32;
        let end = start + first.len() as u32;
        assert!(
            page.spans
                .iter()
                .all(|span| span.end <= start || span.start >= end),
            "{first}"
        );
    }
    page.validate().unwrap();

    // TSV: commas and quotes are ordinary text, and each line is one record.
    let text = "name\tversion\tnote\tnext\r\nab_glyph\t0.2.32\t\"tab\there, and comma\"\r\n";
    let page = source_page("inventory.TSV", text);
    // The decoder shows CRLF as one line break.
    assert_eq!(page.text, text.replace("\r\n", "\n"));
    for (token, role) in [
        ("version", STYLE_STRING),
        ("note", STYLE_NUMBER),
        ("next", STYLE_FUNCTION),
        ("0.2.32", STYLE_STRING),
        ("\"tab", STYLE_NUMBER),
        ("here, and comma\"", STYLE_FUNCTION),
    ] {
        styled(&page.text, &page.spans, token, role);
    }
    assert!(page.spans.iter().all(|span| {
        !page.text[span.start as usize..span.end as usize].contains(['\t', '\r', '\n'])
    }));
    page.validate().unwrap();
}

#[test]
fn a_page_that_starts_inside_a_quoted_csv_field_keeps_its_column() {
    let text = "id,note,n\n1,\"first line\nsecond, line\nthird\",2\n3,x,4\n";
    let mut host = Host::new(text.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 7,
            length: Some(text.len() as u64),
            mode: MODE_SOURCE,
            display_name: "notes.csv".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    // The first page records the lexical state at each complete line.
    let first = page_at(&mut host, 0);
    styled(
        &first.text,
        &first.spans,
        "first line\nsecond, line\nthird",
        STYLE_STRING,
    );
    let anchor = text.find("second").unwrap();
    let page = page_at(&mut host, anchor as u64);
    assert_eq!(page.text, &text[anchor..]);
    styled(&page.text, &page.spans, "second, line\nthird", STYLE_STRING);
    styled(&page.text, &page.spans, "2", STYLE_NUMBER);
    styled(&page.text, &page.spans, "x", STYLE_STRING);
    styled(&page.text, &page.spans, "4", STYLE_NUMBER);
    page.validate().unwrap();
}

#[test]
fn csv_and_tsv_extensions_route_to_their_profiles() {
    let csv = syntax::detect("data.csv", "").expect("CSV profile");
    let tsv = syntax::detect("data.tsv", "").expect("TSV profile");
    assert_ne!(csv, tsv);
    assert_eq!(syntax::detect("DATA.CSV", ""), Some(csv));
    assert_eq!(syntax::detect("data.Tsv", ""), Some(tsv));
    for name in ["data.csv.gz", "data.psv", "data.tab", "data.xlsx"] {
        assert_eq!(syntax::detect(name, ""), None, "{name}");
    }
    let extensions = include_str!("../plugin.toml")
        .lines()
        .find(|line| line.starts_with("extensions ="))
        .unwrap();
    for extension in ["csv", "tsv"] {
        assert_eq!(
            extensions.matches(&format!("\"{extension}\"")).count(),
            1,
            "{extension} needs default routing"
        );
    }
}

#[test]
fn lua_long_brackets_match_their_level_across_complete_lines() {
    let mut state = State::default();
    let first = "--[==[ comment\n";
    styled(
        first,
        &lex("a.lua", first, &mut state),
        first,
        STYLE_COMMENT,
    );
    let middle = "]=] return 'still a comment'\n";
    styled(
        middle,
        &lex("a.lua", middle, &mut state),
        middle,
        STYLE_COMMENT,
    );
    let last = "]==] local s = [=[long\n";
    let spans = lex("a.lua", last, &mut state);
    styled(last, &spans, "]==]", STYLE_COMMENT);
    styled(last, &spans, "local", STYLE_KEYWORD);
    styled(last, &spans, "[=[long\n", STYLE_STRING);
    let last = "string ]] -- inert\n";
    styled(last, &lex("a.lua", last, &mut state), last, STYLE_STRING);
    let last = "]=] return 3\n";
    let spans = lex("a.lua", last, &mut state);
    styled(last, &spans, "]=]", STYLE_STRING);
    styled(last, &spans, "return", STYLE_KEYWORD);
}

#[test]
fn yaml_keys_plain_values_urls_and_flow_collections_are_coloured() {
    let text = "name: midnight\nurl: https://host/path#fragment\nflow: {answer: 42, enabled: true}\nlegacy: yes\n";
    let spans = lex("a.yaml", text, &mut State::default());
    for key in ["name", "url", "flow", "answer", "enabled", "legacy"] {
        styled(text, &spans, key, STYLE_TYPE);
    }
    for value in ["midnight", "https://host/path#fragment", "yes"] {
        styled(text, &spans, value, STYLE_STRING);
    }
    styled(text, &spans, "42", STYLE_NUMBER);
    styled(text, &spans, "true", STYLE_KEYWORD);
    assert!(
        !spans.iter().any(|s| s.role == STYLE_COMMENT),
        "hash inside plain scalar is not a comment"
    );
}

#[test]
fn yaml_block_scalars_keep_blank_lines_and_end_at_dedent() {
    for header in ["literal: |\n", "folded: >-\n", "explicit: |2+\n"] {
        let mut state = State::default();
        lex("a.yml", header, &mut state);
        for text in ["\n", "  true: # still a string\n", "\n", "  return 42\n"] {
            let spans = lex("a.yml", text, &mut state);
            if !text.trim().is_empty() {
                styled(text, &spans, text, STYLE_STRING);
            }
        }
        let text = "next: false # comment\n";
        let spans = lex("a.yml", text, &mut state);
        styled(text, &spans, "next", STYLE_TYPE);
        styled(text, &spans, "false", STYLE_KEYWORD);
        styled(text, &spans, "# comment", STYLE_COMMENT);
    }
}

#[test]
fn yaml_single_quotes_use_doubled_quotes_and_preserve_backslashes() {
    let text = "value: 'can''t \\'\nafter: 42\n";
    let spans = lex("a.yaml", text, &mut State::default());
    styled(text, &spans, "'can''t \\'", STYLE_STRING);
    styled(text, &spans, "after", STYLE_TYPE);
    styled(text, &spans, "42", STYLE_NUMBER);
}

#[test]
fn language_state_is_unchanged_when_cancelled() {
    for (name, text) in [("a.lua", "--[=[long\n"), ("a.yaml", "key: |\n")] {
        let language = syntax::detect(name, "").expect("recognized");
        let mut state = State::default();
        let original = state;
        assert!(syntax::highlight(text, language, &mut state, || true).is_none());
        assert_eq!(state, original);
    }
}

#[test]
fn lua_z_escape_keeps_the_short_string_open_across_whitespace_lines() {
    let mut state = State::default();
    lex("a.lua", "local s = \"a\\z  \n", &mut state);
    let text = "  b\"; return 7\n";
    let spans = lex("a.lua", text, &mut state);
    styled(text, &spans, "  b\"", STYLE_STRING);
    styled(text, &spans, "return", STYLE_KEYWORD);
}

#[test]
fn cpp_custom_delimiter_raw_string_state_ignores_quotes_and_comments() {
    let mut state = State::default();
    lex("a.cpp", "auto s = u8R\"tag(first \" /*\n", &mut state);
    let text = "class 42 // still literal\n";
    styled(text, &lex("a.cpp", text, &mut state), text, STYLE_STRING);
    let text = ")tag\"; return 7;\n";
    let spans = lex("a.cpp", text, &mut state);
    styled(text, &spans, ")tag\"", STYLE_STRING);
    styled(text, &spans, "return", STYLE_KEYWORD);
}

#[test]
fn dense_yaml_span_limits_shorten_both_directions_without_losing_all_styles() {
    let text = format!("[{}0]\n", "1,".repeat(4096));
    for backwards in [false, true] {
        let mut host = Host::new(text.as_bytes(), entry);
        host.submit(
            abi::REQUEST_OPEN,
            &Open {
                reader: 1,
                revision: 7,
                length: Some(text.len() as u64),
                mode: MODE_SOURCE,
                display_name: "dense.yaml".into(),
            },
        );
        host.finish();
        let id = host.submit(
            abi::REQUEST_VIEWPORT,
            &Viewport {
                source_anchor: if backwards { text.len() as u64 } else { 0 },
                backwards,
                max_bytes: 8192,
                max_lines: 512,
            },
        );
        let events = host.finish();
        let page: TextPage = codec::decode(
            &events
                .iter()
                .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
                .expect("dense styled YAML")
                .payload,
        )
        .unwrap();
        assert!(!page.spans.is_empty(), "direction {backwards}");
        assert!(page.spans.len() <= MAX_SPANS);
        assert!(!page.text.is_empty());
        if backwards {
            assert!(text.ends_with(&page.text));
            assert_eq!(page.source.end().unwrap(), text.len() as u64);
        } else {
            assert!(text.starts_with(&page.text));
            assert_eq!(page.source.offset, 0);
        }
        page.validate().unwrap();
    }
}
