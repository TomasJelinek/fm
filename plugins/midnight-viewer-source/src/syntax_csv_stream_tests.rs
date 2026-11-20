use super::*;
use crate::styles::Styles;
use std::cell::Cell;

fn run_with(
    text: &str,
    dialect: Dialect,
    ends: impl IntoIterator<Item = usize>,
) -> (State, Vec<Span>) {
    let mut state = State::default();
    let mut styles = Styles::new(0..text.len(), false);
    let mut at = 0;
    for end in ends {
        styles.origin(at);
        let consumed = stream(
            &text[at..end],
            dialect,
            &mut state,
            &mut styles,
            end == text.len(),
            || false,
        )
        .expect("bounded CSV streaming");
        assert!(text.is_char_boundary(at + consumed));
        at += consumed;
        // Only a quote inside a quoted field waits for the next turn.
        assert!(end - at <= 1, "unbounded ambiguous CSV suffix");
        assert!(text[at..end].bytes().all(|byte| byte == b'"'));
    }
    assert_eq!(at, text.len());
    (state, styles.finish().0)
}

fn run(text: &str, ends: impl IntoIterator<Item = usize>) -> (State, Vec<Span>) {
    run_with(text, Dialect::Csv, ends)
}

/// The one role of `start..end`: the span covering all of it, or plain when no span
/// touches it. A range split between roles fails.
fn role_at(spans: &[Span], start: usize, end: usize) -> u16 {
    let touching: Vec<_> = spans
        .iter()
        .filter(|span| (span.start as usize) < end && span.end as usize > start)
        .collect();
    match touching[..] {
        [] => STYLE_PLAIN,
        [span] if span.start as usize <= start && span.end as usize >= end => span.role,
        _ => panic!("{start}..{end} has mixed roles: {spans:?}"),
    }
}

/// The roles of successive `fields`, each searched for after the previous one.
fn roles(text: &str, spans: &[Span], fields: &[&str]) -> Vec<u16> {
    let mut from = 0;
    fields
        .iter()
        .map(|field| {
            let start = from
                + text[from..]
                    .find(field)
                    .unwrap_or_else(|| panic!("{field:?} after byte {from}"));
            from = start + field.len();
            role_at(spans, start, from)
        })
        .collect()
}

/// Every byte at which `structural` holds stays plain.
fn assert_plain(text: &str, spans: &[Span], structural: impl Fn(usize) -> bool) {
    for at in (0..text.len()).filter(|&at| structural(at)) {
        assert_eq!(
            role_at(spans, at, at + 1),
            STYLE_PLAIN,
            "byte {at} of {text:?}"
        );
    }
}

/// The documented roles of the first eleven columns: plain, string, number, function
/// and type, repeating.
const COLUMN_ROLES: [u16; 11] = [
    STYLE_PLAIN,
    STYLE_STRING,
    STYLE_NUMBER,
    STYLE_FUNCTION,
    STYLE_TYPE,
    STYLE_PLAIN,
    STYLE_STRING,
    STYLE_NUMBER,
    STYLE_FUNCTION,
    STYLE_TYPE,
    STYLE_PLAIN,
];

const RECORDS: &str =
    "id,name,price,qty,city,country,code\n1,apple,0.5,10,Prague,CZ,A1\n2,pear,0.75,3,Brno,CZ,B2\n";
/// The first records of the project's `docs/third-party-inventory.csv`: eleven quoted
/// columns, empty quoted fields and commas inside fields.
const INVENTORY: &str = r#""Package","Version","Direct","License","WindowsFeatures","LinuxFeatures","WindowsGraph","LinuxGraph","PackagedRootNotices","Repository","PinnedSource"
"ab_glyph","0.2.32","False","Apache-2.0","","default,gvar-alloc,std,variable-fonts","False","True","LICENSE","https://github.com/alexheretic/ab-glyph","https://docs.rs/crate/ab_glyph/0.2.32/source/"
"ab_glyph_rasterizer","0.1.10","False","Apache-2.0","","default,std","False","True","LICENSE","https://github.com/alexheretic/ab-glyph","https://docs.rs/crate/ab_glyph_rasterizer/0.1.10/source/"
"adler2","2.0.1","False","0BSD OR MIT OR Apache-2.0","","","True","True","LICENSE-0BSD;LICENSE-APACHE;LICENSE-MIT","https://github.com/oyvindln/adler2","https://docs.rs/crate/adler2/2.0.1/source/"
"#;
const QUOTED: &str = "id,name,quote,note,n\r\n7,\"Smith, J.\",\"He said \"\"hi\"\"\",\"line one\nline two\",1\r\n8,\"a\r\nb\",x,\"\",2\r\n";
const EMPTY: &str = "a,,c,\n,e,,\n\n\"\",g\n";
const MALFORMED: &str = "0,a\"b,c\n1,x\"y\"z,w\n2,\"q\"r,s\n3,\"open,never\nclosed,t";
const LINE_BREAKS: &str = "a,b\rc,d\r\ne,f\n\r\ng,h";
const UNICODE: &str = "naïve,日本,😀\n\"é,\"\"ü\"\"\",ß,€\u{301}\n";
const TSV_RECORDS: &str = "id\tname\tprice, EUR\tqty\tcity\tcountry\tcode\r\n1\tapple\t0,50\t10\tPrague\tCZ\tA1\r\n2\tpear\t0,75\t3\tBrno\tCZ\tB2\r\n";
const TSV_EMPTY: &str = "a\t\tc\t\n\te\t\t\n\n\t\tg\r\n";
const TSV_QUOTES: &str = "0\t\"a\tb\"\tc\n1\t\"open\tx\n2\ty\t\"\"z\"\"\n";

#[test]
fn csv_stream_matches_every_scalar_split_and_single_scalar_turns() {
    for (dialect, text) in [
        (Dialect::Csv, RECORDS),
        (Dialect::Csv, INVENTORY),
        (Dialect::Csv, QUOTED),
        (Dialect::Csv, EMPTY),
        (Dialect::Csv, MALFORMED),
        (Dialect::Csv, LINE_BREAKS),
        (Dialect::Csv, UNICODE),
        (Dialect::Csv, "x,\""),
        (Dialect::Csv, "x,\"\""),
        (Dialect::Csv, "x,\"b\""),
        (Dialect::Csv, "x,\"b\"\""),
        (Dialect::Csv, "\"\"\"\",\"\"\"\"\"\"\n"),
        (Dialect::Tsv, TSV_RECORDS),
        (Dialect::Tsv, TSV_EMPTY),
        (Dialect::Tsv, TSV_QUOTES),
        (Dialect::Tsv, QUOTED),
        (Dialect::Tsv, MALFORMED),
        (Dialect::Tsv, "x\t\""),
    ] {
        let expected = run_with(text, dialect, [text.len()]);
        let boundaries: Vec<_> = text.char_indices().map(|(at, _)| at).skip(1).collect();
        for &split in &boundaries {
            assert_eq!(
                run_with(text, dialect, [split, text.len()]),
                expected,
                "split {split}: {text:?}"
            );
        }
        assert_eq!(
            run_with(text, dialect, boundaries.into_iter().chain([text.len()])),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn csv_fields_take_the_role_of_their_column_and_delimiters_stay_plain() {
    let text = RECORDS;
    let (state, spans) = run(text, [text.len()]);
    assert_eq!(state, State::default());
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let mut start = line_start;
        // The header and every record: the sixth and seventh columns repeat the first two.
        for (column, field) in line.trim_end().split(',').enumerate() {
            assert_eq!(
                role_at(&spans, start, start + field.len()),
                COLUMN_ROLES[column],
                "{field:?} in {line:?}"
            );
            start += field.len() + 1;
        }
        line_start += line.len();
    }
    assert_plain(text, &spans, |at| {
        matches!(text.as_bytes()[at], b',' | b'\n')
    });
}

#[test]
fn inventory_records_colour_eleven_quoted_columns_and_keep_quotes_and_delimiters_plain() {
    let text = INVENTORY;
    let (state, spans) = run(text, [text.len()]);
    assert_eq!(state, State::default());
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let record = line.trim_end();
        let fields: Vec<_> = record[1..record.len() - 1].split("\",\"").collect();
        assert_eq!(fields.len(), 11, "{record}");
        let mut start = line_start + 1;
        for (column, field) in fields.into_iter().enumerate() {
            // Commas inside `default,gvar-alloc,...` do not start columns: the fields
            // after it keep their own column's role.
            if !field.is_empty() {
                assert_eq!(
                    role_at(&spans, start, start + field.len()),
                    COLUMN_ROLES[column],
                    "{field:?}"
                );
            }
            start += field.len() + 3;
        }
        line_start += line.len();
    }
    let bytes = text.as_bytes();
    assert_plain(text, &spans, |at| {
        bytes[at] == b'"'
            || (bytes[at] == b',' && bytes[at - 1] == b'"' && bytes.get(at + 1) == Some(&b'"'))
    });
}

#[test]
fn quoted_fields_keep_delimiters_doubled_quotes_and_line_breaks_in_their_column() {
    let text = QUOTED;
    let (state, spans) = run(text, [text.len()]);
    assert_eq!(state, State::default());
    assert_eq!(
        roles(
            text,
            &spans,
            &[
                "id",
                "name",
                "quote",
                "note",
                "n",
                "7",
                "Smith, J.",
                "He said \"\"hi\"\"",
                "line one\nline two",
                "1",
                "8",
                "a\r\nb",
                "x",
                "2",
            ]
        ),
        [
            STYLE_PLAIN,
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_FUNCTION,
            STYLE_TYPE,
            STYLE_PLAIN,
            STYLE_STRING,
            STYLE_NUMBER,
            // The column continues across the line breaks inside the quotes.
            STYLE_FUNCTION,
            STYLE_TYPE,
            STYLE_PLAIN,
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_TYPE,
        ]
    );
    // The quotes around each quoted field are plain; the doubled ones inside are not.
    for (open, close) in [
        ("\"Smith", "J.\""),
        ("\"He said", "hi\"\"\""),
        ("\"line one", "two\""),
        ("\"a\r\n", "b\""),
    ] {
        let open = text.find(open).unwrap();
        let close = open + text[open..].find(close).unwrap() + close.len() - 1;
        assert_eq!(text.as_bytes()[close], b'"');
        assert_eq!(role_at(&spans, open, open + 1), STYLE_PLAIN, "{open}");
        assert_eq!(role_at(&spans, close, close + 1), STYLE_PLAIN, "{close}");
    }
    let empty = text.find("\"\",2").unwrap();
    assert_eq!(role_at(&spans, empty, empty + 2), STYLE_PLAIN);
}

#[test]
fn empty_fields_trailing_delimiters_and_blank_lines_keep_later_columns() {
    let text = EMPTY;
    let (state, spans) = run(text, [text.len()]);
    assert_eq!(state, State::default());
    assert_eq!(
        roles(text, &spans, &["a", "c", "e", "g"]),
        [STYLE_PLAIN, STYLE_NUMBER, STYLE_STRING, STYLE_STRING]
    );
    // Empty fields, including the empty quoted one, have no span.
    assert_eq!(spans.len(), 3, "{spans:?}");
}

#[test]
fn lf_cr_and_crlf_end_records_outside_quotes() {
    let text = LINE_BREAKS;
    let (_, spans) = run(text, [text.len()]);
    assert_eq!(
        roles(text, &spans, &["a", "b", "c", "d", "e", "f", "g", "h"]),
        [STYLE_PLAIN, STYLE_STRING].repeat(4)
    );
}

#[test]
fn malformed_quotes_stay_in_their_field_and_the_next_record_recovers() {
    let text = MALFORMED;
    let (state, spans) = run(text, [text.len()]);
    assert_eq!(
        roles(
            text,
            &spans,
            &[
                // A quote inside an unquoted field is literal.
                "a\"b",
                "c",
                "x\"y\"z",
                "w",
                // Text after a closing quote stays in its field.
                "q",
                "r",
                "s",
                // An unterminated quote keeps its field, line breaks and delimiters
                // included, to the end of the input.
                "open,never\nclosed,t",
            ]
        ),
        [
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_STRING,
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_STRING,
        ]
    );
    let closing = text.find("q\"r").unwrap() + 1;
    assert_eq!(role_at(&spans, closing, closing + 1), STYLE_PLAIN);
    assert_eq!(
        state,
        State {
            field: Field::Quoted,
            column: 1,
        }
    );
}

#[test]
fn unicode_fields_keep_whole_scalars() {
    let text = UNICODE;
    let (_, spans) = run(text, [text.len()]);
    for span in &spans {
        assert!(text.is_char_boundary(span.start as usize));
        assert!(text.is_char_boundary(span.end as usize));
    }
    assert_eq!(
        roles(
            text,
            &spans,
            &["naïve", "日本", "😀", "é,\"\"ü\"\"", "ß", "€\u{301}"]
        ),
        [
            STYLE_PLAIN,
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_PLAIN,
            STYLE_STRING,
            STYLE_NUMBER,
        ]
    );
}

#[test]
fn tsv_fields_take_the_role_of_their_column_and_tabs_and_crlf_stay_plain() {
    let text = TSV_RECORDS;
    let (state, spans) = run_with(text, Dialect::Tsv, [text.len()]);
    assert_eq!(state, State::default());
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let mut start = line_start;
        // Commas are ordinary text; the sixth and seventh columns repeat the first two.
        for (column, field) in line.trim_end().split('\t').enumerate() {
            assert_eq!(
                role_at(&spans, start, start + field.len()),
                COLUMN_ROLES[column],
                "{field:?} in {line:?}"
            );
            start += field.len() + 1;
        }
        line_start += line.len();
    }
    assert_plain(text, &spans, |at| {
        matches!(text.as_bytes()[at], b'\t' | b'\r' | b'\n')
    });
}

#[test]
fn tsv_empty_fields_trailing_tabs_and_blank_lines_keep_later_columns() {
    let text = TSV_EMPTY;
    let (state, spans) = run_with(text, Dialect::Tsv, [text.len()]);
    assert_eq!(state, State::default());
    assert_eq!(
        roles(text, &spans, &["a", "c", "e", "g"]),
        [STYLE_PLAIN, STYLE_NUMBER, STYLE_STRING, STYLE_NUMBER]
    );
    assert_eq!(spans.len(), 3, "{spans:?}");
}

#[test]
fn tsv_quotes_are_ordinary_text_and_never_join_lines() {
    let text = TSV_QUOTES;
    let (state, spans) = run_with(text, Dialect::Tsv, [text.len()]);
    assert_eq!(state, State::default());
    assert_eq!(
        roles(
            text,
            &spans,
            &[
                "0",
                "\"a",
                "b\"",
                "c",
                "1",
                "\"open",
                "x",
                "2",
                "y",
                "\"\"z\"\"",
            ]
        ),
        [
            STYLE_PLAIN,
            // A quote at the start of a field opens nothing: the tab after `"a` ends it.
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_FUNCTION,
            STYLE_PLAIN,
            // An unclosed quote ends with its line.
            STYLE_STRING,
            STYLE_NUMBER,
            STYLE_PLAIN,
            STYLE_STRING,
            STYLE_NUMBER,
        ]
    );
    // The same bytes as CSV show the difference: the quoted CSV field spans the tab.
    let (_, spans) = run_with("0,\"a\tb\",c\n", Dialect::Csv, [10]);
    assert_eq!(role_at(&spans, 3, 6), STYLE_STRING);
}

#[test]
fn eleven_column_records_of_1500_characters_stream_in_small_turns() {
    let features = (0..80)
        .map(|feature| format!("Win32_Feature_{feature:02}"))
        .collect::<Vec<_>>()
        .join(",");
    let record = format!(
        "\"windows\",\"0.62.2\",\"True\",\"MIT OR Apache-2.0\",\"{features}\",\"\",\"True\",\"False\",\"LICENSE-APACHE;LICENSE-MIT\",\"https://github.com/microsoft/windows-rs\",\"https://docs.rs/crate/windows/0.62.2/source/\"\n"
    );
    assert!(record.len() > 1500);
    let header = INVENTORY.split_inclusive('\n').next().unwrap();
    let text = format!("{header}{}", record.repeat(300));
    // Turns of 4093 bytes cut fields, quotes and delimiters at every phase.
    let ends: Vec<_> = (1..)
        .map(|turn| turn * 4093)
        .take_while(|&end| end < text.len())
        .chain([text.len()])
        .collect();
    let (state, spans) = run(&text, ends);
    assert_eq!(state, State::default());
    // Eight of the eleven columns have a coloured role and are never empty here.
    assert_eq!(spans.len(), 8 * 301);
    let last = text.rfind(&features).unwrap();
    assert_eq!(
        role_at(&spans, last, last + features.len()),
        STYLE_TYPE,
        "the feature list is one field"
    );
    let source = text.rfind("https://docs.rs").unwrap();
    assert_eq!(role_at(&spans, source, text.len() - 2), STYLE_PLAIN);
    let repository = text.rfind("https://github.com").unwrap();
    assert_eq!(
        role_at(&spans, repository, repository + 5),
        STYLE_TYPE,
        "tenth column"
    );
}

#[test]
fn csv_stream_consumes_huge_fields_without_a_token_buffer() {
    for (dialect, prefix, chunk, field, column) in [
        (Dialect::Csv, "", "x".repeat(65_536), Field::Unquoted, 0),
        (
            Dialect::Csv,
            "a,\"",
            ",\n\"\"".repeat(16_384),
            Field::Quoted,
            1,
        ),
        (Dialect::Csv, "a,b", "€".repeat(21_845), Field::Unquoted, 1),
        // A TSV field that starts with a quote is ordinary text of any length.
        (
            Dialect::Tsv,
            "a\tb\t",
            "\"x,".repeat(21_845),
            Field::Unquoted,
            2,
        ),
    ] {
        let mut state = State::default();
        let mut styles = Styles::new(0..0, false);
        assert_eq!(
            stream(prefix, dialect, &mut state, &mut styles, false, || false),
            Some(prefix.len())
        );
        for _ in 0..20 {
            assert_eq!(
                stream(&chunk, dialect, &mut state, &mut styles, false, || false),
                Some(chunk.len()),
                "{prefix:?}"
            );
        }
        assert_eq!(state, State { field, column }, "{prefix:?}");
    }
    assert!(std::mem::size_of::<State>() <= 2);
}

#[test]
fn csv_stream_cancellation_is_transactional_and_bound_is_enforced() {
    let mut state = State::default();
    stream(
        "a,\"b",
        Dialect::Csv,
        &mut state,
        &mut Styles::new(0..0, false),
        false,
        || false,
    );
    let before = state;
    assert_eq!(before.field, Field::Quoted);
    // One quoted field without a closing quote, then many short fields.
    for chunk in ["x,".repeat(32_768), format!("\",{}", "x,".repeat(32_767))] {
        let calls = Cell::new(0);
        assert!(
            stream(
                &chunk,
                Dialect::Csv,
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
    }
    assert!(
        stream(
            &"x".repeat(65_537),
            Dialect::Csv,
            &mut state,
            &mut Styles::new(0..0, false),
            false,
            || false
        )
        .is_none()
    );
    assert_eq!(state, before);
}

/// Deterministic xorshift soup of CSV punctuation, fields, line breaks and Unicode.
fn soup(seed: u64, length: usize) -> String {
    const PIECES: [&str; 24] = [
        "a", "bc", "1", "2.5", ",", ",", ",,", "\t", "\"", "\"\"", "\"a,b\"", "\"\t\"", "\n",
        "\r\n", "\r", " ", ";", "é", "😀", "\"\n", ",\"", "\"\r", "\\", "\u{301}",
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
fn csv_random_soup_never_panics_and_is_split_invariant() {
    for seed in 0..64 {
        let text = soup(seed, 600);
        for dialect in [Dialect::Csv, Dialect::Tsv] {
            let expected = run_with(&text, dialect, [text.len()]);
            for span in &expected.1 {
                assert!(
                    [STYLE_STRING, STYLE_NUMBER, STYLE_FUNCTION, STYLE_TYPE].contains(&span.role)
                );
                assert!(text.is_char_boundary(span.start as usize));
                assert!(text.is_char_boundary(span.end as usize));
                // A TSV field, quotes and all, never holds a tab or a line break.
                let styled = &text[span.start as usize..span.end as usize];
                assert!(dialect == Dialect::Csv || !styled.contains(['\t', '\r', '\n']));
            }
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
                assert_eq!(
                    run_with(&text, dialect, ends.clone()),
                    expected,
                    "{dialect:?} {ends:?}: {text:?}"
                );
            }
        }
    }
}
