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
        .expect("bounded make streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 32, "unbounded ambiguous make suffix");
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

/// The role covering `token`, searched for after `context` (or from the start).
fn role(text: &str, spans: &[Span], context: &str, token: &str) -> u16 {
    let base = text
        .find(context)
        .unwrap_or_else(|| panic!("{context:?} in fixture"));
    let start = base
        + text[base..]
            .find(token)
            .unwrap_or_else(|| panic!("{token:?} after {context:?}"));
    let end = start + token.len();
    spans
        .iter()
        .find(|span| span.start as usize <= start && span.end as usize >= end)
        .map_or(STYLE_PLAIN, |span| span.role)
}

const BUILD: &str = "# Build everything \\\n  still a comment\nCC := gcc\nCFLAGS ?= -O2 -Wall\nLDLIBS += -lm\nSRC = $(wildcard src/*.c)\nOBJ = $(patsubst %.c,%.o,$(SRC))\n.PHONY: all clean\nall: app\n\napp: $(OBJ) | build\n\t$(CC) $(CFLAGS) -o $@ $^ $(LDLIBS)\n\n%.o: %.c\n\t@echo \"CC $<\"\n\t-$(CC) -c $< -o $@ ; true\n\t# shell comment\nclean: ; rm -f $(OBJ) app\n";
const DIRECTIVES_FIXTURE: &str = "ifeq ($(OS),Windows_NT)\n\tEXE := .exe\nelse ifdef MINGW\n  EXE =\nendif\n-include deps.mk\nexport PATH := $(PWD)/bin:$(PATH)\noverride define RECIPE =\n\techo $(1) # not a comment\ndefine nested\nendef\nendef\nundefine RECIPE\nvpath %.h include\n";
const EDGES: &str = "a\\#b: c\\#d # real comment\r\nx y &: z\r\n\trecipe ${VAR} $$HOME \\\\\r\n\tthird \\\r\ncontinued recipe\r\nd :: e\r\nVAR :::= value\r\nSHELLOUT != date\r\ntarget_name_longer_than_thirty_two_bytes_for_sure another: dep\r\n$(foo $(bar ($(baz)) } x\r\nü.o: ü.c\r\n";
const FIXTURES: [&str; 6] = [
    "all:\n\t@echo done\n",
    BUILD,
    DIRECTIVES_FIXTURE,
    EDGES,
    "x\\\n# not a line start\ninclude\\\n a.mk\n\\\\\n$\n$x$(\n${a)b}\n#\\\\\nendif",
    "define a\n\tbody $(x\nendef extra\n\t$(call f,$(g))\n.SILENT .PHONY: y\n.PHONYX: z\n:\n=\n&\n?\n",
];

#[test]
fn make_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for text in FIXTURES {
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
fn make_targets_variables_references_and_recipes_have_roles() {
    let text = FIXTURES[0];
    let spans = run(text, [text.len()]).1;
    assert_eq!(role(text, &spans, "", "all"), STYLE_FUNCTION);
    assert_eq!(role(text, &spans, "\t", "@"), STYLE_KEYWORD);
    assert_eq!(role(text, &spans, "", "echo done"), STYLE_STRING);

    let text = BUILD;
    let spans = run(text, [text.len()]).1;
    for (context, token, expected) in [
        (
            "",
            "# Build everything \\\n  still a comment",
            STYLE_COMMENT,
        ),
        ("CC :=", "CC", STYLE_TYPE),
        ("CC :=", ":=", STYLE_PLAIN),
        ("CC :=", "gcc", STYLE_PLAIN),
        ("CFLAGS ?=", "CFLAGS", STYLE_TYPE),
        ("CFLAGS ?=", "?=", STYLE_PLAIN),
        ("LDLIBS +=", "LDLIBS", STYLE_TYPE),
        ("SRC =", "SRC", STYLE_TYPE),
        ("SRC =", "$(", STYLE_TYPE),
        ("SRC =", "wildcard", STYLE_FUNCTION),
        ("SRC =", "src/*.c", STYLE_PLAIN),
        ("SRC =", "c)", STYLE_PLAIN),
        ("OBJ =", "patsubst", STYLE_FUNCTION),
        ("OBJ =", "$(SRC))", STYLE_TYPE),
        (".PHONY", ".PHONY", STYLE_KEYWORD),
        (".PHONY", "all clean", STYLE_PLAIN),
        ("all: app", "all", STYLE_FUNCTION),
        ("all: app", "app", STYLE_PLAIN),
        ("app: $(OBJ)", "app", STYLE_FUNCTION),
        ("app: $(OBJ)", "$(OBJ)", STYLE_TYPE),
        ("app: $(OBJ)", "| build", STYLE_PLAIN),
        ("\t$(CC)", "$(CC)", STYLE_TYPE),
        ("\t$(CC)", " -o ", STYLE_STRING),
        ("\t$(CC)", "$@", STYLE_TYPE),
        ("\t$(CC)", "$^", STYLE_TYPE),
        ("%.o:", "%.o", STYLE_FUNCTION),
        ("\t@echo", "@", STYLE_KEYWORD),
        ("\t@echo", "echo \"CC ", STYLE_STRING),
        ("\t@echo", "$<", STYLE_TYPE),
        ("\t-$(CC)", "-", STYLE_KEYWORD),
        ("\t-$(CC)", " ; true", STYLE_STRING),
        ("\t# shell", "# shell comment", STYLE_COMMENT),
        ("clean:", "clean", STYLE_FUNCTION),
        ("clean:", "rm -f ", STYLE_STRING),
        ("clean:", "$(OBJ)", STYLE_TYPE),
        ("clean:", " app", STYLE_STRING),
    ] {
        assert_eq!(
            role(text, &spans, context, token),
            expected,
            "{context:?} {token:?}: {spans:?}"
        );
    }
}

#[test]
fn make_directives_conditionals_and_define_bodies_have_roles() {
    let text = DIRECTIVES_FIXTURE;
    let spans = run(text, [text.len()]).1;
    for (context, token, expected) in [
        ("", "ifeq", STYLE_KEYWORD),
        ("ifeq", "$(OS)", STYLE_TYPE),
        ("ifeq", "Windows_NT", STYLE_PLAIN),
        // A tab outside a rule's recipe is indentation.
        ("\tEXE", "EXE", STYLE_TYPE),
        ("else", "else", STYLE_KEYWORD),
        ("else", "ifdef", STYLE_KEYWORD),
        ("else", "MINGW", STYLE_TYPE),
        ("  EXE =", "EXE", STYLE_TYPE),
        ("endif", "endif", STYLE_KEYWORD),
        ("-include", "-include", STYLE_KEYWORD),
        ("-include", "deps.mk", STYLE_PLAIN),
        ("export", "export", STYLE_KEYWORD),
        ("export", "PATH", STYLE_TYPE),
        ("export", "$(PWD)", STYLE_TYPE),
        ("export", "/bin:", STYLE_PLAIN),
        ("override", "override", STYLE_KEYWORD),
        ("override", "define", STYLE_KEYWORD),
        ("override", "RECIPE", STYLE_TYPE),
        ("\techo", "\techo ", STYLE_STRING),
        ("\techo", "$(1)", STYLE_TYPE),
        ("\techo", " # not a comment", STYLE_STRING),
        ("define nested", "define nested", STYLE_STRING),
        ("define nested\nendef", "endef", STYLE_STRING),
        ("define nested\nendef\nendef", "\nendef\n", STYLE_PLAIN),
        ("undefine", "undefine", STYLE_KEYWORD),
        ("undefine", "RECIPE", STYLE_TYPE),
        ("vpath", "vpath", STYLE_KEYWORD),
        ("vpath", "%.h include", STYLE_PLAIN),
    ] {
        assert_eq!(
            role(text, &spans, context, token),
            expected,
            "{context:?} {token:?}: {spans:?}"
        );
    }
    let last = text.rfind("endef").unwrap();
    assert!(spans.iter().any(|span| span.role == STYLE_KEYWORD
        && span.start as usize == last
        && span.end as usize == last + 5));
    let (state, _) = run(text, [text.len()]);
    assert_eq!((state.defines, state.depth, state.rule), (0, 0, false));
}

#[test]
fn make_escapes_continuations_separators_and_unicode_have_roles() {
    let text = EDGES;
    let spans = run(text, [text.len()]).1;
    for (context, token, expected) in [
        ("", "a\\#b", STYLE_FUNCTION),
        ("", "c\\#d", STYLE_PLAIN),
        ("", "# real comment", STYLE_COMMENT),
        ("x y", "x y", STYLE_FUNCTION),
        ("x y", "z", STYLE_PLAIN),
        ("\trecipe", "recipe ", STYLE_STRING),
        ("\trecipe", "${VAR}", STYLE_TYPE),
        ("\trecipe", "$$", STYLE_TYPE),
        ("\trecipe", "HOME \\\\", STYLE_STRING),
        // A single backslash continues the recipe; an escaped one does not.
        ("\tthird", "continued recipe", STYLE_STRING),
        ("d ::", "d", STYLE_FUNCTION),
        ("d ::", "e", STYLE_PLAIN),
        ("VAR :::=", "VAR", STYLE_TYPE),
        ("VAR :::=", "value", STYLE_PLAIN),
        ("SHELLOUT", "SHELLOUT", STYLE_TYPE),
        (
            "target_name",
            "target_name_longer_than_thirty_two_bytes_for_sure another",
            STYLE_FUNCTION,
        ),
        ("$(foo", "$(foo", STYLE_TYPE),
        ("$(foo", "$(bar", STYLE_TYPE),
        ("$(foo", "$(baz))", STYLE_PLAIN),
        ("$(foo", "$(baz)", STYLE_TYPE),
        ("$(foo", "} x", STYLE_PLAIN),
        ("ü.o", "ü.o", STYLE_FUNCTION),
        ("ü.o", "ü.c", STYLE_PLAIN),
    ] {
        assert_eq!(
            role(text, &spans, context, token),
            expected,
            "{context:?} {token:?}: {spans:?}"
        );
    }
    // The escaped backslash ends that recipe line: `third` starts a new one.
    let third = text.find("\tthird").unwrap() + 1;
    assert!(
        spans
            .iter()
            .any(|span| span.role == STYLE_STRING && span.start as usize == third)
    );
}

#[test]
fn make_stream_consumes_giant_heads_recipes_and_comments_without_a_token_buffer() {
    for (prefix, chunk, pending) in [
        ("", "x".repeat(65_536), true),
        ("a: b\n\t", "y".repeat(65_536), false),
        ("# ", "z".repeat(65_536), false),
        ("V = $(", "w".repeat(65_536), false),
        ("define x\n", "v".repeat(65_536), false),
    ] {
        let mut state = State::default();
        let mut styles = Styles::new(0..0, false);
        assert_eq!(
            stream(prefix, &mut state, &mut styles, false, || false),
            Some(prefix.len())
        );
        for _ in 0..20 {
            assert_eq!(
                stream(&chunk, &mut state, &mut styles, false, || false),
                Some(chunk.len()),
                "{prefix:?}"
            );
        }
        assert_eq!(state.pending_style(), pending, "{prefix:?}");
    }
    let mut state = State::default();
    let mut styles = Styles::new(0..70_000, false);
    stream(&"t".repeat(65_536), &mut state, &mut styles, false, || {
        false
    });
    assert_eq!(state.pending_span(), Some((65_536, 0)));
    styles.origin(65_536);
    stream("  ", &mut state, &mut styles, false, || false);
    assert_eq!(state.pending_span(), Some((65_538, 2)));
    styles.origin(65_538);
    assert_eq!(
        stream(": d", &mut state, &mut styles, false, || false),
        Some(3)
    );
    assert!(!state.pending_style());
    assert!(
        styles
            .finish()
            .0
            .iter()
            .any(|span| span.role == STYLE_FUNCTION && span.start == 0 && span.end == 65_536)
    );
    assert!(std::mem::size_of::<State>() <= 64);
}

#[test]
fn make_reference_nesting_saturates_and_resets_at_the_line_end() {
    let text = format!("{}{}\nall: x\n\tok\n", "$(a ".repeat(40), ")".repeat(40));
    let (state, spans) = run(&text, [text.len()]);
    assert_eq!((state.depth, state.groups), (0, 0));
    let ok = text.find("ok").unwrap();
    assert!(
        spans
            .iter()
            .any(|span| span.role == STYLE_STRING && span.start as usize == ok)
    );
}

#[test]
fn make_stream_cancellation_is_transactional_and_bound_is_enforced() {
    let mut state = State::default();
    stream(
        "all",
        &mut state,
        &mut Styles::new(0..0, false),
        false,
        || false,
    );
    let before = state;
    assert!(before.pending_style());
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

/// Deterministic xorshift soup of make punctuation, words and Unicode.
fn soup(seed: u64, length: usize) -> String {
    const PIECES: [&str; 42] = [
        "a", "b", "all", "x.o", ".PHONY", "include", "define", "endef", "ifeq", "else", "export",
        "$", "$(", "${", "$$", "$@", "(", ")", "{", "}", ":", "::", ":=", "=", "?=", "+", "!", "&",
        ";", "#", "\\", "@", "-", " ", "\t", "\n", "\r\n", "\n\t", "é", "😀", ",", "%",
    ];
    let mut state = seed | 1;
    let mut text = String::new();
    while text.len() < length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push_str(PIECES[(state % PIECES.len() as u64) as usize]);
    }
    text
}

#[test]
fn make_random_soup_never_panics_and_is_split_invariant() {
    for seed in 0..64 {
        let text = soup(seed, 600);
        let expected = run(&text, [text.len()]);
        let boundaries: Vec<_> = text.char_indices().map(|(at, _)| at).collect();
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        for _ in 0..24 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut ends: Vec<usize> = (0..4)
                .map(|shift| boundaries[((state >> (shift * 16)) as usize) % boundaries.len()])
                .collect();
            ends.sort_unstable();
            ends.dedup();
            ends.retain(|end| *end != 0);
            ends.push(text.len());
            assert_eq!(run(&text, ends.clone()), expected, "{ends:?}: {text:?}");
        }
    }
}

#[test]
fn a_recipe_continues_after_else_with_crlf_blanks_or_a_comment() {
    for ending in ["\n", "\r\n", " \n", " # note\n", " # note\r\n"] {
        let text = format!("all:\nifeq (a,b)\n\techo a\nelse{ending}\techo b\nendif\n");
        let expected = run(&text, [text.len()]);
        assert_eq!(
            role(&text, &expected.1, "\techo b", "echo b"),
            STYLE_STRING,
            "{ending:?}: {:?}",
            expected.1
        );
        for split in 1..text.len() {
            assert_eq!(
                run(&text, [split, text.len()]),
                expected,
                "{ending:?} split {split}"
            );
        }
    }
}
