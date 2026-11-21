//! Bounded CSV and TSV colouring in the manner of Rainbow CSV: each field takes the role
//! of its column, so the columns of long records stay easy to follow. Fields are never
//! collected, typed or validated. A `.csv` file is separated by commas or semicolons,
//! decided once from its first record ([`Dialect::of_csv`]).
use midnight_plugin_sdk::api::viewer::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dialect {
    /// Comma-separated values with RFC 4180 quoting: a field that starts with a quote may
    /// contain commas, doubled quotes and line breaks.
    Csv,
    /// Semicolon-separated values with the same quoting, as spreadsheets save CSV in
    /// locales whose decimal separator is the comma. Commas are ordinary text.
    Semicolon,
    /// Tab-separated values in the IANA `text/tab-separated-values` form: a record is one
    /// line and quotes are ordinary text, so a stray quote cannot swallow later lines.
    Tsv,
}

impl Dialect {
    /// The dialect of a `.csv` file whose decoded text starts with `prefix`. Only the
    /// first record decides, because later records of a semicolon file hold decimal
    /// commas: it is split once at semicolons and once at commas, each with its own
    /// RFC 4180 quoting, so a delimiter inside a quoted field counts for neither. More
    /// fields at semicolons make the file semicolon-separated; a tie, a record with
    /// neither delimiter and an empty prefix keep the comma of RFC 4180. A spreadsheet's
    /// `sep=;` first line therefore selects semicolons too.
    pub(super) fn of_csv(prefix: &str) -> Self {
        if delimiters(prefix, b';') > delimiters(prefix, b',') {
            Self::Semicolon
        } else {
            Self::Csv
        }
    }

    fn delimiter(self) -> u8 {
        match self {
            Self::Csv => b',',
            Self::Semicolon => b';',
            Self::Tsv => b'\t',
        }
    }

    /// Whether a quote that starts a field opens a quoted field.
    fn quoting(self) -> bool {
        self != Self::Tsv
    }
}

/// The `delimiter`s that separate fields of the first record of `text`: the bytes before
/// the first line break outside quotes, with the quoting that [`stream`] applies.
fn delimiters(text: &str, delimiter: u8) -> usize {
    let bytes = text.as_bytes();
    let mut field = Field::Start;
    let mut count = 0;
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        at += 1;
        if field == Field::Quoted {
            if byte == b'"' {
                // A doubled quote is content; a single one closes the field.
                if bytes.get(at) == Some(&b'"') {
                    at += 1;
                } else {
                    field = Field::Unquoted;
                }
            }
        } else if byte == delimiter {
            count += 1;
            field = Field::Start;
        } else if matches!(byte, b'\n' | b'\r') {
            break;
        } else if byte == b'"' && field == Field::Start {
            field = Field::Quoted;
        } else {
            field = Field::Unquoted;
        }
    }
    count
}

/// Field roles by column, repeating after the last. The first column keeps the text
/// colour. The others avoid the theme accent, which the default themes make grey, and
/// are ordered so that neighbouring columns differ clearly on dark and light
/// backgrounds: the similar orange `NUMBER` and amber `TYPE` are never neighbours.
const COLUMNS: [u16; 5] = [
    STYLE_PLAIN,
    STYLE_STRING,
    STYLE_NUMBER,
    STYLE_FUNCTION,
    STYLE_TYPE,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    field: Field,
    /// The current field's column, modulo the number of column roles.
    column: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Field {
    /// Before the first byte of a field, where a CSV quote opens a quoted field.
    #[default]
    Start,
    /// An unquoted field, or what follows a closing quote before the next delimiter
    /// (malformed, but kept in its field). Quotes here are literal.
    Unquoted,
    /// Between CSV quotes: delimiters, line breaks and doubled quotes are content.
    Quoted,
}

impl State {
    fn role(self) -> u16 {
        COLUMNS[usize::from(self.column)]
    }
}

pub(super) fn scan(
    text: &str,
    dialect: Dialect,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, dialect, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// Consume a bounded decoded turn. Fields of any length are consumed as runs, never
/// buffered. Delimiters and the quotes around a quoted field stay plain; a record ends
/// at an unquoted LF, CR or CRLF. Only a quote that ends a non-final turn inside a
/// quoted CSV field stays with the caller, until the next byte tells a closing quote
/// from the first of a doubled one.
pub(super) fn stream(
    text: &str,
    dialect: Dialect,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    let mut poll = super::Poll::new(cancelled);
    if text.len() > 65_536 || poll.cancelled(0) {
        return None;
    }
    let delimiter = dialect.delimiter();
    let mut current = *state;
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        let start = at;
        let byte = bytes[at];
        if current.field == Field::Quoted {
            if byte != b'"' {
                at = run(text, at, |byte| byte == b'"');
                styles.push(start, at, current.role());
                continue;
            }
            match bytes.get(at + 1) {
                None if !final_chunk => break,
                Some(b'"') => {
                    at += 2;
                    styles.push(start, at, current.role());
                }
                // A closing quote; anything before the next delimiter stays in the field.
                _ => {
                    at += 1;
                    current.field = Field::Unquoted;
                }
            }
        } else if byte == delimiter {
            at += 1;
            current.column = (current.column + 1) % COLUMNS.len() as u8;
            current.field = Field::Start;
        } else if matches!(byte, b'\n' | b'\r') {
            // Each byte of a CRLF ends a record; the second ends an empty one.
            at += 1;
            current = State::default();
        } else if byte == b'"' && current.field == Field::Start && dialect.quoting() {
            at += 1;
            current.field = Field::Quoted;
        } else {
            at = run(text, at, |byte| {
                byte == delimiter || matches!(byte, b'\n' | b'\r')
            });
            styles.push(start, at, current.role());
            current.field = Field::Unquoted;
        }
    }
    if poll.cancelled(at) {
        return None;
    }
    *state = current;
    Some(at)
}

/// The end of the run from `at` up to the first byte after it that `stop` accepts, at
/// most `POLL_BYTES` long and on a character boundary, so that cancellation is polled
/// in long fields. The byte at `at` never stops the run, which is therefore not empty.
fn run(text: &str, at: usize, stop: impl Fn(u8) -> bool) -> usize {
    let mut end = text.len().min(at + super::POLL_BYTES);
    if let Some(length) = text.as_bytes()[at + 1..end]
        .iter()
        .position(|&byte| stop(byte))
    {
        return at + 1 + length;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
#[path = "syntax_csv_stream_tests.rs"]
mod stream_tests;
