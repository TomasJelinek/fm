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

fn lex(name: &str, text: &str, state: &mut State) -> Vec<Span> {
    let language = syntax::detect(name, "").expect("recognized language");
    syntax::highlight(text, language, state, || false).expect("bounded lexical output")
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
