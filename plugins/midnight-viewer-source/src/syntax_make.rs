//! Bounded GNU make lexical colouring: nothing is expanded, included or run.
use midnight_plugin_sdk::api::viewer::*;

/// Directives recognised as the first word of a line (or after a modifier).
const DIRECTIVES: [&[u8]; 18] = [
    b"include",
    b"-include",
    b"sinclude",
    b"ifeq",
    b"ifneq",
    b"ifdef",
    b"ifndef",
    b"else",
    b"endif",
    b"define",
    b"endef",
    b"undefine",
    b"export",
    b"unexport",
    b"override",
    b"private",
    b"vpath",
    b"load",
];
const ALL_DIRECTIVES: u32 = (1 << DIRECTIVES.len()) - 1;
const SPECIAL_TARGETS: [&[u8]; 17] = [
    b".PHONY",
    b".SUFFIXES",
    b".DEFAULT",
    b".PRECIOUS",
    b".INTERMEDIATE",
    b".SECONDARY",
    b".SECONDEXPANSION",
    b".DELETE_ON_ERROR",
    b".IGNORE",
    b".LOW_RESOLUTION_TIME",
    b".SILENT",
    b".EXPORT_ALL_VARIABLES",
    b".NOTPARALLEL",
    b".ONESHELL",
    b".POSIX",
    b".NOTINTERMEDIATE",
    b".EXTRA_PREREQS",
];
const ALL_SPECIAL_TARGETS: u32 = (1 << SPECIAL_TARGETS.len()) - 1;
const FUNCTIONS: [&[u8]; 39] = [
    b"abspath",
    b"addprefix",
    b"addsuffix",
    b"and",
    b"basename",
    b"call",
    b"dir",
    b"error",
    b"eval",
    b"file",
    b"filter",
    b"filter-out",
    b"findstring",
    b"firstword",
    b"flavor",
    b"foreach",
    b"guile",
    b"if",
    b"info",
    b"intcmp",
    b"join",
    b"lastword",
    b"let",
    b"notdir",
    b"or",
    b"origin",
    b"patsubst",
    b"realpath",
    b"shell",
    b"sort",
    b"strip",
    b"subst",
    b"suffix",
    b"value",
    b"warning",
    b"wildcard",
    b"word",
    b"wordlist",
    b"words",
];
/// Longest function name: a longer reference name is a variable.
const MAX_FUNCTION: usize = 10;
/// Open references and brackets inside them, two bits each in `State::groups`.
const MAX_DEPTH: u8 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    line: Line,
    mode: Mode,
    /// Per open group: bit 0 a `$(`/`${` reference, bit 1 brace brackets.
    groups: u64,
    depth: u8,
    /// Open `define` bodies.
    defines: u8,
    /// A rule precedes: a line starting with a tab is its recipe.
    rule: bool,
    /// This logical line is a `define`: its body starts on the next line.
    opening: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Line {
    /// The start of a logical line.
    #[default]
    Start,
    /// Leading blanks of a line that is not a recipe.
    Lead,
    /// The first words of a line, whose role depends on what follows.
    Head(Head),
    /// The variable name after `define`, `undefine`, `ifdef` or `ifndef`.
    Name {
        started: bool,
    },
    /// Values, prerequisites and directive arguments.
    Text {
        prerequisites: bool,
    },
    Recipe {
        prefix: bool,
    },
    RecipeComment,
    Comment,
    /// A line of a `define` body.
    Body {
        start: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    /// Right after `$(` or `${`: a function or variable name follows.
    Reference,
    /// The rest of a variable name inside a reference.
    Variable,
}

/// The consumed head region: its length including trailing blanks, which are
/// not styled, and matchers for its first word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Head {
    length: u64,
    trailing: u64,
    words: u8,
    position: u8,
    directives: u32,
    specials: u32,
}

impl Head {
    fn new(directives: bool) -> Self {
        Self {
            length: 0,
            trailing: 0,
            words: 0,
            position: 0,
            directives: if directives { ALL_DIRECTIVES } else { 0 },
            specials: ALL_SPECIAL_TARGETS,
        }
    }

    /// A region that continues after a reference or a line continuation.
    fn rest() -> Self {
        Self {
            directives: 0,
            specials: 0,
            ..Self::new(false)
        }
    }

    fn pending(&self) -> bool {
        self.length > self.trailing
    }

    /// The directive that the complete first word names.
    fn directive(&self) -> Option<usize> {
        (self.words == 1)
            .then(|| exact(self.directives, self.position, &DIRECTIVES))
            .flatten()
    }

    fn special(&self) -> bool {
        self.words == 1 && exact(self.specials, self.position, &SPECIAL_TARGETS).is_some()
    }
}

impl State {
    pub(super) fn pending_style(&self) -> bool {
        self.pending_span().is_some()
    }

    /// The head region's consumed length and excluded trailing blanks, relative to
    /// the cursor, while its role depends on following input.
    pub(super) fn pending_span(&self) -> Option<(u64, u64)> {
        match self.line {
            Line::Head(head) if head.pending() => Some((head.length, head.trailing)),
            _ => None,
        }
    }

    fn push_group(&mut self, reference: bool, brace: bool) {
        if self.depth < MAX_DEPTH {
            let shift = 2 * u32::from(self.depth);
            self.groups &= !(3 << shift);
            self.groups |= (u64::from(reference) | u64::from(brace) << 1) << shift;
            self.depth += 1;
        }
    }

    /// The innermost group: (reference, brace).
    fn top(&self) -> Option<(bool, bool)> {
        let depth = self.depth.checked_sub(1)?;
        let bits = self.groups >> (2 * u32::from(depth));
        Some((bits & 1 != 0, bits & 2 != 0))
    }

    fn pop_group(&mut self) {
        if let Some(depth) = self.depth.checked_sub(1) {
            self.groups &= !(3 << (2 * u32::from(depth)));
            self.depth = depth;
        }
    }

    /// An unescaped newline ends the logical line.
    fn end_line(&mut self) {
        if self.opening {
            self.defines = self.defines.saturating_add(1);
            self.opening = false;
        }
        self.groups = 0;
        self.depth = 0;
        self.mode = Mode::Normal;
        self.line = Line::Start;
    }

    /// The head ended without a rule or assignment: a directive, or plain text.
    fn end_head(&mut self, head: Head, at: usize, styles: &mut crate::styles::Styles) {
        if let Some(directive) = head.directive() {
            styles.push_pending(head.length, head.trailing, at, STYLE_KEYWORD);
            self.directive(directive);
            return;
        }
        // Nothing but blanks, a CR or a comment (as after `else`): like blank and
        // comment-only lines, it neither ends a rule's recipe nor starts text.
        if head.length == 0 {
            return;
        }
        if head.pending() {
            styles.push_pending(head.length, head.trailing, at, STYLE_PLAIN);
        }
        self.rule = false;
        self.line = Line::Text {
            prerequisites: false,
        };
    }

    fn directive(&mut self, index: usize) {
        // Conditionals keep a preceding rule's recipe context.
        if !(3..=8).contains(&index) {
            self.rule = false;
        }
        self.line = match DIRECTIVES[index] {
            b"ifdef" | b"ifndef" | b"define" | b"undefine" => Line::Name { started: false },
            b"else" | b"export" | b"unexport" | b"override" | b"private" => {
                Line::Head(Head::new(true))
            }
            _ => Line::Text {
                prerequisites: false,
            },
        };
        if DIRECTIVES[index] == b"define" {
            self.opening = true;
        }
    }
}

fn exact(candidates: u32, position: u8, words: &[&[u8]]) -> Option<usize> {
    words
        .iter()
        .enumerate()
        .find(|(index, word)| candidates & (1 << index) != 0 && word.len() == usize::from(position))
        .map(|(index, _)| index)
}

fn narrow(candidates: u32, position: u8, byte: u8, words: &[&[u8]]) -> u32 {
    let mut remaining = candidates;
    for (index, word) in words.iter().enumerate() {
        if word.get(usize::from(position)) != Some(&byte) {
            remaining &= !(1 << index);
        }
    }
    remaining
}

fn blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c')
}

fn variable_char(byte: u8) -> bool {
    !blank(byte)
        && !matches!(
            byte,
            b'\n' | b'$' | b'(' | b')' | b'{' | b'}' | b':' | b',' | b'=' | b'#' | b'\\'
        )
}

/// The byte at `offset` after `at`: `Some(None)` at the end of the final chunk,
/// `None` when the next chunk must decide.
fn peek(bytes: &[u8], at: usize, offset: usize, final_chunk: bool) -> Option<Option<u8>> {
    match bytes.get(at + offset) {
        Some(byte) => Some(Some(*byte)),
        None if final_chunk => Some(None),
        None => None,
    }
}

/// A backslash at `at` that ends the physical line: the bytes of the backslash
/// and line break, or 0. `None`: more input is needed.
fn continuation(bytes: &[u8], at: usize, final_chunk: bool) -> Option<usize> {
    Some(match peek(bytes, at, 1, final_chunk)? {
        Some(b'\n') => 2,
        Some(b'\r') => match peek(bytes, at, 2, final_chunk)? {
            Some(b'\n') => 3,
            _ => 0,
        },
        _ => 0,
    })
}

enum Dollar {
    Plain,
    /// `$x`, `$@` or `$$`: its bytes.
    Single(usize),
    Open {
        brace: bool,
    },
}

/// What a `$` at `at` starts. `None`: more input is needed.
fn dollar(text: &str, at: usize, final_chunk: bool) -> Option<Dollar> {
    Some(match peek(text.as_bytes(), at, 1, final_chunk)? {
        None | Some(b'\n' | b'\r') => Dollar::Plain,
        Some(b'(') => Dollar::Open { brace: false },
        Some(b'{') => Dollar::Open { brace: true },
        Some(_) => Dollar::Single(1 + text.get(at + 1..)?.chars().next()?.len_utf8()),
    })
}

/// The colons of a `:=`, `::=` or `:::=` operator at `at`; `Some(None)` for a rule
/// separator. `None`: more input is needed.
fn assignment_colons(bytes: &[u8], at: usize, final_chunk: bool) -> Option<Option<usize>> {
    let mut colons = 1;
    loop {
        match peek(bytes, at, colons, final_chunk)? {
            Some(b':') if colons < 3 => colons += 1,
            next => return Some((next == Some(b'=')).then_some(colons)),
        }
    }
}

/// Whether `word` followed by a blank, a line break, a comment or the end starts
/// the define body line at `at`. `None`: more input is needed.
fn body_word(bytes: &[u8], at: usize, word: &[u8], final_chunk: bool) -> Option<bool> {
    for (offset, expected) in word.iter().enumerate() {
        match peek(bytes, at, offset, final_chunk)? {
            Some(byte) if byte == *expected => {}
            _ => return Some(false),
        }
    }
    Some(match peek(bytes, at, word.len(), final_chunk)? {
        None => true,
        Some(byte) => blank(byte) || matches!(byte, b'\n' | b'#'),
    })
}

pub(super) fn scan(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// Consume a bounded decoded turn. Heads and names of any length keep counters,
/// never text; at most a function name or an operator stays with the caller.
pub(super) fn stream(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    let mut poll = super::Poll::new(cancelled);
    if text.len() > 65_536 || poll.cancelled(0) {
        return None;
    }
    let mut current = *state;
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        let start = at;
        let byte = bytes[at];
        let ch = text[at..].chars().next()?;
        let width = ch.len_utf8();
        match current.mode {
            Mode::Reference => {
                let mut length = 0;
                let terminator = loop {
                    match peek(bytes, at, length, final_chunk) {
                        None => break None,
                        Some(Some(next)) if variable_char(next) && length < MAX_FUNCTION => {
                            length += 1;
                        }
                        Some(next) => break Some(next),
                    }
                };
                let Some(terminator) = terminator else {
                    break;
                };
                // A known function name followed by a blank; anything else names a variable.
                if terminator.is_some_and(|next| next == b' ' || next == b'\t')
                    && FUNCTIONS.contains(&&bytes[at..at + length])
                {
                    at += length;
                    styles.push(start, at, STYLE_FUNCTION);
                    current.mode = Mode::Normal;
                } else {
                    current.mode = Mode::Variable;
                }
                continue;
            }
            Mode::Variable => {
                if variable_char(byte) {
                    at += width;
                    styles.push(start, at, STYLE_TYPE);
                } else {
                    current.mode = Mode::Normal;
                }
                continue;
            }
            Mode::Normal => {}
        }
        let base = match current.line {
            Line::Recipe { .. } | Line::Body { .. } if current.depth == 0 => STYLE_STRING,
            _ => STYLE_PLAIN,
        };
        match current.line {
            Line::Start => {
                if current.defines > 0 {
                    current.line = Line::Body { start: true };
                } else if byte == b'\n' {
                    at += 1;
                } else if byte == b'\t' && current.rule {
                    at += 1;
                    current.line = Line::Recipe { prefix: true };
                } else if byte == b'#' {
                    at += 1;
                    styles.push(start, at, STYLE_COMMENT);
                    current.line = Line::Comment;
                } else if blank(byte) {
                    at += 1;
                    current.line = Line::Lead;
                } else {
                    current.line = Line::Head(Head::new(true));
                }
                continue;
            }
            Line::Lead => {
                if byte == b'\n' {
                    at += 1;
                    current.end_line();
                } else if byte == b'#' {
                    at += 1;
                    styles.push(start, at, STYLE_COMMENT);
                    current.line = Line::Comment;
                } else if blank(byte) {
                    at += 1;
                } else {
                    current.line = Line::Head(Head::new(true));
                }
                continue;
            }
            Line::Comment | Line::RecipeComment => {
                if byte == b'\n' {
                    at += 1;
                    current.end_line();
                    continue;
                }
                let mut length = width;
                if byte == b'\\' {
                    let (Some(next), Some(continued)) = (
                        peek(bytes, at, 1, final_chunk),
                        continuation(bytes, at, final_chunk),
                    ) else {
                        break;
                    };
                    if continued > 0 {
                        at += continued;
                        styles.push(start, at, STYLE_COMMENT);
                        if current.line == Line::RecipeComment {
                            // The shell comment ends here; the recipe continues.
                            current.line = Line::Recipe { prefix: false };
                        }
                        continue;
                    }
                    // An escaped backslash cannot continue the line.
                    if next == Some(b'\\') {
                        length = 2;
                    }
                }
                at += length;
                styles.push(start, at, STYLE_COMMENT);
                continue;
            }
            Line::Body { start: true } if current.depth == 0 => {
                if blank(byte) {
                    at += 1;
                    styles.push(start, at, STYLE_STRING);
                    continue;
                }
                let Some(end) = body_word(bytes, at, b"endef", final_chunk) else {
                    break;
                };
                let Some(nested) = body_word(bytes, at, b"define", final_chunk) else {
                    break;
                };
                current.line = Line::Body { start: false };
                if end && current.defines <= 1 {
                    at += 5;
                    styles.push(start, at, STYLE_KEYWORD);
                    current.defines = 0;
                    current.line = Line::Text {
                        prerequisites: false,
                    };
                } else if end {
                    at += 5;
                    styles.push(start, at, STYLE_STRING);
                    current.defines -= 1;
                } else if nested {
                    at += 6;
                    styles.push(start, at, STYLE_STRING);
                    current.defines = current.defines.saturating_add(1);
                }
                continue;
            }
            _ => {}
        }
        // Line breaks, continuations and escapes.
        let body = matches!(current.line, Line::Body { .. });
        if byte == b'\n' {
            if let Line::Head(head) = current.line {
                current.end_head(head, at, styles);
            }
            at += 1;
            if body {
                current.groups = 0;
                current.depth = 0;
                current.line = Line::Start;
            } else {
                current.end_line();
            }
            continue;
        }
        if byte == b'\\' && !body {
            let (Some(next), Some(length)) = (
                peek(bytes, at, 1, final_chunk),
                continuation(bytes, at, final_chunk),
            ) else {
                break;
            };
            if length > 0 {
                if let Line::Head(head) = current.line {
                    if head.directive().is_some() && head.trailing == 0 {
                        current.end_head(head, at, styles);
                    } else {
                        if head.pending() {
                            styles.push_pending(head.length, head.trailing, at, STYLE_PLAIN);
                        }
                        current.line = Line::Head(Head::rest());
                    }
                }
                if let Line::Name { started: true } = current.line {
                    current.line = Line::Text {
                        prerequisites: false,
                    };
                }
                if let Line::Recipe { .. } = current.line {
                    current.line = Line::Recipe { prefix: false };
                }
                at += length;
                styles.push(start, at, base);
                continue;
            }
            // An escaped `#` outside recipes, or an escaped backslash, which then
            // cannot continue the line, is two literal bytes.
            let recipe = matches!(current.line, Line::Recipe { .. });
            let length = match next {
                Some(b'#') if !recipe => 2,
                Some(b'\\') => 2,
                _ => 1,
            };
            if let Line::Head(mut head) = current.line
                && current.depth == 0
            {
                word(&mut head, byte, 1)?;
                if let Some(escaped) = next.filter(|_| length == 2) {
                    word(&mut head, escaped, 1)?;
                }
                current.line = Line::Head(head);
                at += length;
                continue;
            }
            if let Line::Name { .. } = current.line
                && current.depth == 0
            {
                at += length;
                styles.push(start, at, STYLE_TYPE);
                current.line = Line::Name { started: true };
                continue;
            }
            if let Line::Recipe { .. } = current.line {
                current.line = Line::Recipe { prefix: false };
            }
            at += length;
            styles.push(start, at, base);
            continue;
        }
        // References.
        if byte == b'$' {
            let Some(kind) = dollar(text, at, final_chunk) else {
                break;
            };
            if let Line::Head(head) = current.line {
                if head.pending() {
                    styles.push_pending(head.length, head.trailing, at, STYLE_PLAIN);
                }
                current.line = Line::Head(Head::rest());
            }
            match &mut current.line {
                Line::Name { started } => *started = true,
                Line::Recipe { prefix } => *prefix = false,
                _ => {}
            }
            match kind {
                Dollar::Plain => {
                    at += 1;
                    styles.push(start, at, base);
                }
                Dollar::Single(length) => {
                    at += length;
                    styles.push(start, at, STYLE_TYPE);
                }
                Dollar::Open { brace } => {
                    at += 2;
                    styles.push(start, at, STYLE_TYPE);
                    current.push_group(true, brace);
                    current.mode = Mode::Reference;
                }
            }
            continue;
        }
        // Comments outside recipes and define bodies, even inside references.
        if byte == b'#' && !body && !matches!(current.line, Line::Recipe { .. }) {
            if let Line::Head(head) = current.line {
                current.end_head(head, at, styles);
            }
            at += 1;
            styles.push(start, at, STYLE_COMMENT);
            current.line = Line::Comment;
            continue;
        }
        // Brackets inside references.
        if let Some((_, brace)) = current.top() {
            let (open, close) = if brace { (b'{', b'}') } else { (b'(', b')') };
            if byte == open {
                at += 1;
                current.push_group(false, brace);
                continue;
            }
            if byte == close {
                let reference = current.top().is_some_and(|(reference, _)| reference);
                at += 1;
                if reference {
                    styles.push(start, at, STYLE_TYPE);
                }
                current.pop_group();
                continue;
            }
            at += width;
            continue;
        }
        match current.line {
            Line::Head(mut head) => {
                if blank(byte) {
                    if head.length == 0 {
                        at += 1;
                        continue;
                    }
                    if head.trailing == 0 && head.directive().is_some() {
                        current.end_head(head, at, styles);
                        continue;
                    }
                    head.directives = 0;
                    head.trailing = head.trailing.checked_add(1)?;
                    head.length = head.length.checked_add(1)?;
                    at += 1;
                    current.line = Line::Head(head);
                    continue;
                }
                // Bytes of an assignment operator before its `=`.
                let operator = match byte {
                    b'=' => Some(0),
                    b'?' | b'+' | b'!' => match peek(bytes, at, 1, final_chunk) {
                        None => break,
                        Some(next) => (next == Some(b'=')).then_some(1),
                    },
                    // `:=`, `::=` and `:::=` assign; `:` and `::` separate a rule.
                    b':' => match assignment_colons(bytes, at, final_chunk) {
                        None => break,
                        Some(colons) => colons,
                    },
                    _ => None,
                };
                if let Some(extra) = operator {
                    // An assignment: the variable is the only word before the operator.
                    let role = if head.words == 1 {
                        STYLE_TYPE
                    } else {
                        STYLE_PLAIN
                    };
                    if head.pending() {
                        styles.push_pending(head.length, head.trailing, at, role);
                    }
                    at += extra + 1;
                    current.rule = false;
                    current.line = Line::Text {
                        prerequisites: false,
                    };
                    continue;
                }
                let separator = match byte {
                    b':' => {
                        let double = match peek(bytes, at, 1, final_chunk) {
                            None => break,
                            Some(next) => next == Some(b':'),
                        };
                        Some(if double { 2 } else { 1 })
                    }
                    b'&' => match peek(bytes, at, 1, final_chunk) {
                        None => break,
                        Some(next) => (next == Some(b':')).then_some(2),
                    },
                    _ => None,
                };
                if let Some(length) = separator {
                    let role = if head.special() {
                        STYLE_KEYWORD
                    } else {
                        STYLE_FUNCTION
                    };
                    if head.pending() {
                        styles.push_pending(head.length, head.trailing, at, role);
                    }
                    at += length;
                    current.rule = true;
                    current.line = Line::Text {
                        prerequisites: true,
                    };
                    continue;
                }
                if byte == b';' {
                    current.end_head(head, at, styles);
                    if let Line::Head(_) = current.line {
                        current.line = Line::Text {
                            prerequisites: false,
                        };
                    }
                    continue;
                }
                word(&mut head, byte, width)?;
                at += width;
                current.line = Line::Head(head);
            }
            Line::Name { started } => {
                if blank(byte) {
                    at += 1;
                    if started {
                        current.line = Line::Text {
                            prerequisites: false,
                        };
                    }
                } else if matches!(byte, b'=' | b':' | b'?' | b'+' | b'!') {
                    current.line = Line::Text {
                        prerequisites: false,
                    };
                } else {
                    at += width;
                    styles.push(start, at, STYLE_TYPE);
                    current.line = Line::Name { started: true };
                }
            }
            Line::Text { prerequisites } => {
                at += width;
                if prerequisites && byte == b';' {
                    current.line = Line::Recipe { prefix: true };
                }
            }
            Line::Recipe { prefix: true } => {
                if blank(byte) {
                    at += 1;
                } else if matches!(byte, b'@' | b'-' | b'+') {
                    at += 1;
                    styles.push(start, at, STYLE_KEYWORD);
                } else if byte == b'#' {
                    at += 1;
                    styles.push(start, at, STYLE_COMMENT);
                    current.line = Line::RecipeComment;
                } else {
                    current.line = Line::Recipe { prefix: false };
                }
            }
            Line::Recipe { prefix: false } | Line::Body { .. } => {
                at += width;
                styles.push(start, at, STYLE_STRING);
            }
            Line::Start | Line::Lead | Line::Comment | Line::RecipeComment => {}
        }
    }
    if final_chunk && at == text.len() {
        if let Line::Head(head) = current.line {
            current.end_head(head, at, styles);
        }
        current.mode = Mode::Normal;
    }
    if poll.cancelled(at) {
        return None;
    }
    *state = current;
    Some(at)
}

/// Add a character of `width` bytes to the head region.
fn word(head: &mut Head, byte: u8, width: usize) -> Option<()> {
    if head.trailing > 0 || head.length == 0 {
        head.words = head.words.saturating_add(1);
        head.trailing = 0;
    }
    if head.words == 1 {
        head.directives = narrow(head.directives, head.position, byte, &DIRECTIVES);
        head.specials = narrow(head.specials, head.position, byte, &SPECIAL_TARGETS);
        head.position = head.position.saturating_add(width.min(255) as u8);
    } else {
        head.directives = 0;
        head.specials = 0;
    }
    head.length = head.length.checked_add(width as u64)?;
    Some(())
}

#[cfg(test)]
#[path = "syntax_make_stream_tests.rs"]
mod stream_tests;
