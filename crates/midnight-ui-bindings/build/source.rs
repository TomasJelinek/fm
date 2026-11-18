//! Writes the generated token stream as line-structured Rust source.
//!
//! rustc's `unused_braces` lint builds its suggestion before it consults the lint
//! level. For a `{ expression }` block that starts and ends on one line it copies
//! the whole source before and after the block, even where the lint is allowed. The
//! generated module has thousands of such blocks in tens of megabytes of source, so
//! that copying dominated the compilation of this crate. A block that spans lines is
//! not examined. Every brace therefore ends its line here, as in slint-build's own
//! writer; a width-based formatter such as prettyplease keeps short blocks on one
//! line. Run rustfmt on a copy of the output to read it comfortably.
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use std::fmt::Write;

const INDENT: &str = "    ";

struct Writer {
    text: String,
    depth: usize,
    /// The next token starts a line, unless it continues what the last line closed.
    broken: bool,
}

/// The generated tokens as source text. Fails on an undelimited group, whose
/// grouping this writer could not preserve.
pub fn text(tokens: TokenStream) -> Result<String, String> {
    let mut writer = Writer {
        text: String::new(),
        depth: 0,
        broken: false,
    };
    writer.tokens(tokens, true)?;
    writer.text.push('\n');
    Ok(writer.text)
}

impl Writer {
    fn line(&mut self) {
        self.text.push('\n');
        for _ in 0..self.depth {
            self.text.push_str(INDENT);
        }
    }

    /// `braced` is true for the top level and the contents of a brace group, where a
    /// `;` ends an item or statement and a `,` after a block ends a field or match arm.
    fn tokens(&mut self, tokens: TokenStream, braced: bool) -> Result<(), String> {
        // A joint punctuation character continues in the next token: `::`, `->`, `'a`.
        let mut joint = false;
        let mut block_closed = false;
        for tree in tokens {
            let continues = match &tree {
                TokenTree::Punct(punct) => matches!(punct.as_char(), ';' | ',' | '.' | '?'),
                TokenTree::Ident(ident) => ident == "else",
                _ => false,
            };
            if self.broken && !continues {
                self.line();
            } else if !joint && !self.text.is_empty() && !self.text.ends_with([' ', '\n']) {
                self.text.push(' ');
            }
            self.broken = false;
            joint = false;
            let after_block = std::mem::take(&mut block_closed);
            match tree {
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Brace => ('{', '}'),
                        Delimiter::Parenthesis => ('(', ')'),
                        Delimiter::Bracket => ('[', ']'),
                        Delimiter::None => {
                            return Err("Generated Rust contains an undelimited group".into());
                        }
                    };
                    self.text.push(open);
                    if open != '{' {
                        self.tokens(group.stream(), false)?;
                    } else if !group.stream().is_empty() {
                        self.depth += 1;
                        self.line();
                        self.tokens(group.stream(), true)?;
                        self.depth -= 1;
                        self.line();
                    }
                    self.text.push(close);
                    self.broken = open == '{';
                    block_closed = self.broken;
                }
                TokenTree::Punct(punct) => {
                    self.text.push(punct.as_char());
                    joint = punct.spacing() == Spacing::Joint;
                    self.broken = braced
                        && !joint
                        && (punct.as_char() == ';' || (punct.as_char() == ',' && after_block));
                }
                // Infallible for a String.
                TokenTree::Ident(ident) => write!(self.text, "{ident}").unwrap(),
                TokenTree::Literal(literal) => write!(self.text, "{literal}").unwrap(),
            }
        }
        // A closing delimiter follows on this line.
        self.broken = false;
        Ok(())
    }
}
