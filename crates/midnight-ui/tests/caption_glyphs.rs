//! Every non-ASCII character that the UI's sources can show is drawn by the fonts the
//! application loads, not as the missing-glyph box. Released 0.5.2 showed a box for the
//! Find window's "▾" on Windows: Segoe UI has no such glyph and, being a symbol of the
//! Common script, it gets no script fallback. Alignment checks judge only where a
//! caption's pixels sit, so they passed the box.
//!
//! The check scans the string literals of the `.slint` sources and the non-test Rust
//! sources whose text the windows show, and software-renders each character found with
//! the application's own font family (`Theme.effective-font-family`). The renderer
//! shapes text with the same Slint text stack as the application's renderer (parley
//! over fontique's system collection), so the font chosen for a character, any system
//! fallback (fontconfig on Linux, DirectWrite on Windows) and the missing-glyph box are
//! those of the machine that runs the test. A character whose drawing equals the
//! drawing of a code point no font has (U+10FFFD, private use) is missing. Each
//! character is drawn alone and between Latin letters, because the script fallback
//! follows the surrounding run, at the regular and the bold weight.
use crate::support::modeless as ui;

use slint::{ComponentHandle, platform::software_renderer::MinimalSoftwareWindow};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
};

slint::slint! {
    import { Theme } from "../ui/theme.slint";
    export { Theme }
    export component GlyphProbe inherits Window {
        in property <string> sample;
        in property <int> weight: 400;
        background: white;
        Text {
            x: 4px; y: 4px;
            text: root.sample; color: black;
            font-family: Theme.effective-font-family; font-size: 28px; font-weight: root.weight;
        }
    }
}

const WIDTH: u32 = 220;
const HEIGHT: u32 = 48;
/// A code point no font maps: drawn as the selected font's missing-glyph box.
const MISSING: char = '\u{10FFFD}';
/// The application's UI sources.
const SLINT_FOLDERS: [&str; 3] = [
    "midnight-ui/ui",
    "midnight-viewer-ui/ui",
    "midnight-compare-ui/ui",
];
/// Crates whose texts (captions, labels, status and error messages, key names) the
/// application's windows show in the UI font.
const RUST_FOLDERS: [&str; 11] = [
    "midnight-ui/src",
    "midnight-ui-input/src",
    "midnight-core/src",
    "midnight-engine/src",
    "midnight-platform-common/src",
    "midnight-platform-windows/src",
    "midnight-platform-linux/src",
    "midnight-update/src",
    "midnight-explorer/src",
    "midnight-viewer-ui/src",
    "midnight-compare-ui/src",
];

/// Where each checked character occurs: `file:line`, the first few per character.
type Occurrences = BTreeMap<char, Vec<String>>;

#[test]
fn every_shown_character_has_a_glyph_in_the_application_font() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Occurrences::new();
    for folder in SLINT_FOLDERS {
        for file in files(&crates.join(folder), "slint") {
            let source = std::fs::read_to_string(&file).unwrap();
            record(&mut found, &crates, &file, slint_literals(&source));
        }
    }
    for folder in RUST_FOLDERS {
        for file in files(&crates.join(folder), "rs") {
            if test_only(&file) {
                continue;
            }
            let source = std::fs::read_to_string(&file).unwrap();
            record(&mut found, &crates, &file, rust_literals(&source));
        }
    }
    assert!(
        found.contains_key(&'…'),
        "the scan found no ellipsis: it read no sources"
    );

    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let probe = GlyphProbe::new().unwrap();
    probe.show().unwrap();
    let native = windows.borrow()[0].clone();
    let mut missing = Vec::new();
    for weight in [400, 700] {
        probe.set_weight(weight);
        for (before, after) in [("", ""), ("Ag", "Ag")] {
            let draw = |sample: String| {
                probe.set_sample(sample.into());
                render(&native)
            };
            let reference = draw(format!("{before}{MISSING}{after}"));
            assert_ne!(
                draw(format!("{before}A{after}")),
                reference,
                "weight {weight}: a letter draws like the missing-glyph box; no font loaded"
            );
            // Any other missing code point draws exactly the same.
            assert_eq!(
                draw(format!("{before}\u{10FFFC}{after}")),
                reference,
                "weight {weight}, {before:?}: missing glyphs differ; the check cannot judge"
            );
            for (&character, places) in &found {
                if draw(format!("{before}{character}{after}")) == reference
                    && !missing.iter().any(|(known, _)| *known == character)
                {
                    missing.push((character, places));
                }
            }
        }
    }
    let family = probe.global::<Theme>().get_effective_font_family();
    let report = missing
        .iter()
        .map(|(character, places)| {
            format!(
                "  {character:?} U+{:04X} in {}",
                *character as u32,
                places.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        missing.is_empty(),
        "{} of {} non-ASCII characters draw as the missing-glyph box in {family:?} and its \
         fallbacks on this system; use a bundled icon or a character the font has:\n{report}",
        missing.len(),
        found.len(),
    );
}

fn render(native: &MinimalSoftwareWindow) -> Vec<slint::Rgb8Pixel> {
    ui::render(native, WIDTH, HEIGHT, "caption-glyph")
}

/// Record every visible non-ASCII character of `literals`.
fn record(found: &mut Occurrences, crates: &Path, file: &Path, literals: Vec<(usize, String)>) {
    let name = file
        .strip_prefix(crates)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");
    for (line, literal) in literals {
        for character in literal.chars().filter(|c| !c.is_ascii() && !invisible(*c)) {
            let places = found.entry(character).or_default();
            let place = format!("{name}:{line}");
            if places.len() < 3 && !places.contains(&place) {
                places.push(place);
            }
        }
    }
}

/// Spaces, joiners, direction marks and isolates, variation selectors: nothing to draw.
/// Private use: Slint's codes of named keys (Escape, the arrows), never shown.
fn invisible(character: char) -> bool {
    character.is_whitespace()
        || matches!(character,
            '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}'
            | '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..)
}

fn files(folder: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|found| found == extension) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Test-only Rust files by the workspace's naming: `tests.rs`, `*_tests.rs`, test
/// folders, fixtures and repros. Test modules inside other files are skipped by
/// [`rust_literals`].
fn test_only(file: &Path) -> bool {
    file.components().any(|part| {
        let part = part.as_os_str().to_string_lossy();
        part.contains("test") || part.contains("fixture") || part.contains("repro")
    })
}

/// A character cursor over a source text that counts lines.
struct Cursor {
    text: Vec<char>,
    at: usize,
    line: usize,
}

impl Cursor {
    fn new(source: &str) -> Self {
        Self {
            text: source.chars().collect(),
            at: 0,
            line: 1,
        }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.text.get(self.at + ahead).copied()
    }

    fn starts_with(&self, prefix: &str) -> bool {
        prefix
            .chars()
            .enumerate()
            .all(|(index, c)| self.peek(index) == Some(c))
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek(0)?;
        self.at += 1;
        if character == '\n' {
            self.line += 1;
        }
        Some(character)
    }

    fn skip(&mut self, count: usize) {
        for _ in 0..count {
            self.bump();
        }
    }

    fn line_comment(&mut self) {
        while self.peek(0).is_some_and(|c| c != '\n') {
            self.bump();
        }
    }

    /// A block comment at `/*`; Rust's nest.
    fn block_comment(&mut self, nested: bool) {
        self.skip(2);
        let mut depth = 1;
        while depth > 0 && self.peek(0).is_some() {
            if nested && self.starts_with("/*") {
                depth += 1;
                self.skip(2);
            } else if self.starts_with("*/") {
                depth -= 1;
                self.skip(2);
            } else {
                self.bump();
            }
        }
    }

    /// `\u{…}` at the backslash.
    fn unicode_escape_after_backslash(&mut self) -> Option<char> {
        self.bump();
        self.unicode_escape()
    }

    /// `\u{…}` after the backslash.
    fn unicode_escape(&mut self) -> Option<char> {
        self.skip(2);
        let mut digits = String::new();
        while let Some(c) = self.bump() {
            if c == '}' {
                break;
            }
            digits.push(c);
        }
        u32::from_str_radix(&digits, 16)
            .ok()
            .and_then(char::from_u32)
    }
}

/// The string literals of a `.slint` source with their first lines, comments
/// excluded and escapes decoded. Interpolated expressions (`\{…}`) are code, and the
/// literals inside them are listed separately.
fn slint_literals(source: &str) -> Vec<(usize, String)> {
    let mut cursor = Cursor::new(source);
    let mut literals = Vec::new();
    slint_code(&mut cursor, &mut literals, false);
    literals
}

/// Slint code up to the end, or up to the `}` that closes an interpolation.
fn slint_code(cursor: &mut Cursor, literals: &mut Vec<(usize, String)>, interpolation: bool) {
    let mut depth = 0_usize;
    while let Some(character) = cursor.peek(0) {
        if cursor.starts_with("//") {
            cursor.line_comment();
        } else if cursor.starts_with("/*") {
            cursor.block_comment(false);
        } else if character == '"' {
            slint_string(cursor, literals);
        } else {
            cursor.bump();
            if character == '{' {
                depth += 1;
            } else if character == '}' {
                if depth == 0 && interpolation {
                    return;
                }
                depth = depth.saturating_sub(1);
            }
        }
    }
}

fn slint_string(cursor: &mut Cursor, literals: &mut Vec<(usize, String)>) {
    let line = cursor.line;
    cursor.bump();
    let mut literal = String::new();
    while let Some(character) = cursor.peek(0) {
        match character {
            '"' => {
                cursor.bump();
                break;
            }
            '\\' => match cursor.peek(1) {
                Some('{') => {
                    cursor.skip(2);
                    slint_code(cursor, literals, true);
                }
                Some('u') => literal.extend(cursor.unicode_escape_after_backslash()),
                Some('n') => {
                    cursor.skip(2);
                    literal.push('\n');
                }
                _ => {
                    cursor.bump();
                    literal.extend(cursor.bump());
                }
            },
            _ => literal.extend(cursor.bump()),
        }
    }
    literals.push((line, literal));
}

/// The string and character literals of a Rust source with their first lines,
/// comments and items under a `test` cfg excluded, escapes decoded.
fn rust_literals(source: &str) -> Vec<(usize, String)> {
    let mut cursor = Cursor::new(source);
    let mut literals = Vec::new();
    // Brace and bracket depths; the item after a test cfg ends at a `;` or `,` at its
    // own depth, or with the block that it opened or that encloses it.
    let mut braces = 0_usize;
    let mut groups = 0_usize;
    let mut skipped: Option<(usize, usize)> = None;
    let mut previous = ' ';
    while let Some(character) = cursor.peek(0) {
        let identifier = previous.is_alphanumeric() || previous == '_';
        let literal = if cursor.starts_with("//") {
            cursor.line_comment();
            None
        } else if cursor.starts_with("/*") {
            cursor.block_comment(true);
            None
        } else if !identifier && raw_string_start(&cursor).is_some() {
            Some(rust_raw_string(&mut cursor))
        } else if character == '"' {
            Some(rust_string(&mut cursor))
        } else if character == '\'' {
            rust_character(&mut cursor)
        } else if cursor.starts_with("#![") {
            // A test-only file (`#![cfg(test)]`).
            if test_attribute(&mut cursor) {
                return Vec::new();
            }
            None
        } else if cursor.starts_with("#[") {
            if test_attribute(&mut cursor) && skipped.is_none() {
                skipped = Some((braces, groups));
            }
            None
        } else {
            cursor.bump();
            match character {
                '{' => braces += 1,
                '}' => {
                    braces = braces.saturating_sub(1);
                    if skipped.is_some_and(|(depth, _)| braces <= depth) {
                        skipped = None;
                    }
                }
                '(' | '[' => groups += 1,
                ')' | ']' => groups = groups.saturating_sub(1),
                ';' | ',' if skipped == Some((braces, groups)) => skipped = None,
                _ => {}
            }
            None
        };
        if let Some(literal) = literal
            && skipped.is_none()
        {
            literals.push(literal);
        }
        previous = cursor.at.checked_sub(1).map_or(' ', |at| cursor.text[at]);
    }
    literals
}

/// The number of `#` of a raw string (`r"`, `r#"`, `br"`, `cr#"`) at the cursor.
fn raw_string_start(cursor: &Cursor) -> Option<usize> {
    let mut ahead = usize::from(matches!(cursor.peek(0), Some('b' | 'c')));
    if cursor.peek(ahead) != Some('r') {
        return None;
    }
    ahead += 1;
    let mut hashes = 0;
    while cursor.peek(ahead) == Some('#') {
        hashes += 1;
        ahead += 1;
    }
    (cursor.peek(ahead) == Some('"')).then_some(hashes)
}

fn rust_raw_string(cursor: &mut Cursor) -> (usize, String) {
    let hashes = raw_string_start(cursor).unwrap();
    let line = cursor.line;
    while cursor.bump() != Some('"') {}
    let end = format!("\"{}", "#".repeat(hashes));
    let mut literal = String::new();
    while cursor.peek(0).is_some() && !cursor.starts_with(&end) {
        literal.extend(cursor.bump());
    }
    cursor.skip(end.chars().count());
    (line, literal)
}

fn rust_string(cursor: &mut Cursor) -> (usize, String) {
    let line = cursor.line;
    while cursor.bump() != Some('"') {}
    let mut literal = String::new();
    while let Some(character) = cursor.peek(0) {
        match character {
            '"' => {
                cursor.bump();
                break;
            }
            '\\' => literal.extend(rust_escape(cursor)),
            _ => literal.extend(cursor.bump()),
        }
    }
    (line, literal)
}

/// A character literal at `'`; a lifetime or label yields nothing.
fn rust_character(cursor: &mut Cursor) -> Option<(usize, String)> {
    let line = cursor.line;
    let character = if cursor.peek(1) == Some('\\') {
        cursor.bump();
        rust_escape(cursor)
    } else if cursor.peek(2) == Some('\'') {
        cursor.bump();
        cursor.bump()
    } else {
        cursor.bump();
        return None;
    };
    while cursor.peek(0).is_some_and(|c| c != '\'') {
        cursor.bump();
    }
    cursor.bump();
    Some((line, character.map(String::from).unwrap_or_default()))
}

/// An escape at `\`: the character it stands for, if any.
fn rust_escape(cursor: &mut Cursor) -> Option<char> {
    match cursor.peek(1) {
        Some('u') => cursor.unicode_escape_after_backslash(),
        Some('x') => {
            cursor.skip(4);
            None
        }
        Some('\n' | '\r') => {
            cursor.bump();
            while cursor.peek(0).is_some_and(char::is_whitespace) {
                cursor.bump();
            }
            None
        }
        Some('n') => {
            cursor.skip(2);
            Some('\n')
        }
        _ => {
            cursor.bump();
            cursor.bump()
        }
    }
}

/// An attribute at `#[`/`#![`: whether it limits the next item to tests
/// (`#[test]`, `#[cfg(test)]`, `#[cfg(all(test, …))]`, `#[cfg(any(test, …))]`).
fn test_attribute(cursor: &mut Cursor) -> bool {
    let mut text = String::new();
    let mut depth = 0_usize;
    while let Some(character) = cursor.bump() {
        match character {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '"' => {
                // Attribute strings (paths, doc text) are neither code nor captions.
                cursor.at -= 1;
                rust_string(cursor);
                continue;
            }
            _ => {}
        }
        text.push(character);
    }
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let inner = text.trim_start_matches('#').trim_start_matches('!');
    inner == "[test"
        || (inner.starts_with("[cfg(")
            && !inner.contains("not(test")
            && inner
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == "test"))
}

#[test]
fn the_literal_scanners_find_captions_and_skip_comments_and_tests() {
    let slint = "// \"© comment\"\nText { text: \"▾ \" + \"\\{a ? \"‹\" : \"x\"}·\\u{2192}\"; }\n/* \"✗\" */";
    let found: Vec<String> = slint_literals(slint).into_iter().map(|(_, s)| s).collect();
    assert_eq!(found, ["▾ ", "‹", "x", "·→"]);
    let rust = concat!(
        "// \"© comment\"\nfn a<'a>(x: &'a str) -> char { let _ = r#\"raw ≈\"#; '…' }\n",
        "#[cfg(test)]\nmod tests { const S: &str = \"文档\"; }\n",
        "#[cfg(all(test, windows))]\nmod native;\n",
        "#[cfg(not(test))]\nconst C: &str = \"⚠\";\n",
        "/* outer /* \"nested ✗\" */ \"✗\" */ const D: &str = \"\\u{2014}\\\n    x\";\n"
    );
    let found: Vec<(usize, String)> = rust_literals(rust);
    assert_eq!(
        found,
        [
            (2, "raw ≈".to_owned()),
            (2, "…".to_owned()),
            (8, "⚠".to_owned()),
            (9, "—x".to_owned()),
        ]
    );
}
