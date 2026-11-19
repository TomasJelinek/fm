//! Spreads the generated components over rustc's code generation units.
//!
//! rustc places a method in the code generation unit named after the module of its
//! `Self` type and then merges the smallest units until `codegen-units` remain. It
//! never divides a module. slint's generator writes every component into one module,
//! so one unit held most of this crate and LLVM optimised it on one thread, which
//! was most of the time this crate takes to compile. Measurements:
//! docs/verification/2026-11-19-slint-build-speed.md.
//!
//! Each generated component struct is therefore moved into one of [`PARTS`] private
//! submodules. The `impl` blocks stay where they are: their unit follows the struct.
//! The struct and its fields get `pub(super)` where they had no visibility, which
//! names exactly the scope they were private to, and the parent re-imports every part
//! with a glob, so no path, expression or public item changes.
use proc_macro2::{Delimiter, Group, Ident, Spacing, Span, TokenStream, TokenTree};

/// Submodules the component structs are dealt into. More parts than code generation
/// units cost nothing: rustc merges the smallest units by its own size estimate.
pub const PARTS: usize = 16;

const ITEM_KEYWORDS: [&str; 12] = [
    "struct",
    "enum",
    "union",
    "impl",
    "fn",
    "mod",
    "trait",
    "use",
    "const",
    "static",
    "type",
    "macro_rules",
];
const ITEM_QUALIFIERS: [&str; 5] = ["pub", "unsafe", "async", "default", "extern"];
/// Items of these kinds may contain a brace group before the `;` that ends them.
const ENDS_AT_SEMICOLON: [&str; 4] = ["use", "const", "static", "type"];

/// The generated tokens with the component structs of the generated module dealt
/// into `parts` submodules. Fails on a module item this pass does not know, so that a
/// new generator output is noticed instead of being rearranged wrongly.
pub fn spread(tokens: TokenStream, parts: usize) -> Result<TokenStream, String> {
    let mut output = TokenStream::new();
    // 1 after `mod`, 2 after its name: the brace group that follows is the module.
    let mut header = 0;
    let mut done = false;
    for tree in tokens {
        match tree {
            TokenTree::Group(group)
                if header == 2 && !done && group.delimiter() == Delimiter::Brace =>
            {
                let body = module(group.stream(), parts.max(1))?;
                output.extend([TokenTree::Group(Group::new(Delimiter::Brace, body))]);
                done = true;
            }
            tree => {
                header = match &tree {
                    TokenTree::Ident(_) if header == 1 => 2,
                    TokenTree::Ident(ident) if ident == "mod" => 1,
                    _ => 0,
                };
                output.extend([tree]);
            }
        }
    }
    if done {
        Ok(output)
    } else {
        Err("Generated Rust contains no module".into())
    }
}

fn module(body: TokenStream, parts: usize) -> Result<TokenStream, String> {
    let mut parent = TokenStream::new();
    let mut part: Vec<TokenStream> = (0..parts).map(|_| TokenStream::new()).collect();
    let mut moved = 0;
    for item in items(body)? {
        if is_component_struct(&item) {
            if names_relative_scope(item.iter().cloned().collect()) {
                // `super::T` or `pub(self)` would mean something else one module down.
                return Err("A generated component struct names `self` or `super`".into());
            }
            part[moved % parts].extend(widen_struct(item));
            moved += 1;
        } else {
            parent.extend(item);
        }
    }
    if moved == 0 {
        // The generator's naming changed, or this is not its component output (live
        // preview): say so instead of silently not spreading.
        return Err("Generated Rust contains no `struct Inner…` component".into());
    }
    for (index, part) in part.into_iter().enumerate().take(moved) {
        let name = format!("slint_part_{index}");
        let mut body = fixed("#[allow(unused_imports)] use super::*;");
        body.extend(part);
        parent.extend(fixed("mod"));
        parent.extend([
            TokenTree::Ident(Ident::new(&name, Span::call_site())),
            TokenTree::Group(Group::new(Delimiter::Brace, body)),
        ]);
        // `pub`, so that a struct that was `pub` in the module stays as visible as it
        // was; a glob caps every other item at its own `pub(super)`.
        parent.extend(fixed(&format!(
            "#[allow(unused_imports)] pub use {name}::*;"
        )));
    }
    Ok(parent)
}

fn fixed(text: &str) -> TokenStream {
    text.parse().expect("fixed tokens")
}

/// Splits the contents of a module into its items, attributes included.
fn items(body: TokenStream) -> Result<Vec<Vec<TokenTree>>, String> {
    let mut items = Vec::new();
    let mut item: Vec<TokenTree> = Vec::new();
    let mut keyword: Option<String> = None;
    let mut trees = body.into_iter().peekable();
    while let Some(tree) = trees.next() {
        let mut ends = false;
        match &tree {
            TokenTree::Punct(punct) if keyword.is_none() && punct.as_char() == '#' => {
                // `#[..]` belongs to the item that follows; `#![..]` stands alone.
                if matches!(trees.peek(), Some(TokenTree::Punct(next)) if next.as_char() == '!') {
                    if !item.is_empty() {
                        return Err("Generated Rust has an inner attribute inside an item".into());
                    }
                    item.push(tree);
                    item.extend(trees.next());
                    item.extend(trees.next());
                    items.push(std::mem::take(&mut item));
                    continue;
                }
            }
            TokenTree::Ident(ident) if keyword.is_none() => {
                let word = ident.to_string();
                let before_fn = |next: &Ident| {
                    ["fn", "unsafe", "async", "extern"].contains(&next.to_string().as_str())
                };
                let qualifies_fn = word == "const"
                    && matches!(trees.peek(), Some(TokenTree::Ident(next)) if before_fn(next));
                if ITEM_KEYWORDS.contains(&word.as_str()) && !qualifies_fn {
                    keyword = Some(word);
                } else if !qualifies_fn && !ITEM_QUALIFIERS.contains(&word.as_str()) {
                    return Err(format!(
                        "Generated Rust has an item that starts with `{word}`"
                    ));
                }
            }
            TokenTree::Punct(punct) if keyword.is_some() && punct.as_char() == ';' => ends = true,
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
                let Some(word) = keyword.as_deref() else {
                    return Err("Generated Rust has a block that belongs to no known item".into());
                };
                ends = !ENDS_AT_SEMICOLON.contains(&word);
            }
            _ => {}
        }
        item.push(tree);
        if ends {
            keyword = None;
            items.push(std::mem::take(&mut item));
        }
    }
    if item.is_empty() {
        Ok(items)
    } else {
        Err("Generated Rust ends inside an item".into())
    }
}

/// `struct Inner… { … }`: the generator's name for a component, global or repeated
/// element. A struct declared in Slint is `r#Name` and never matches.
fn is_component_struct(item: &[TokenTree]) -> bool {
    let keyword = item
        .iter()
        .position(|tree| matches!(tree, TokenTree::Ident(ident) if ident == "struct"));
    let named = keyword.and_then(|at| item.get(at + 1)).is_some_and(
        |tree| matches!(tree, TokenTree::Ident(name) if name.to_string().starts_with("Inner")),
    );
    named
        && matches!(item.last(), Some(TokenTree::Group(group))
            if group.delimiter() == Delimiter::Brace)
}

/// Whether the tokens name `self` or `super` at any depth, as a path or a visibility.
fn names_relative_scope(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|tree| match tree {
        TokenTree::Ident(ident) => ident == "self" || ident == "super",
        TokenTree::Group(group) => names_relative_scope(group.stream()),
        _ => false,
    })
}

/// The struct with itself and its fields visible in the module it came from.
fn widen_struct(mut item: Vec<TokenTree>) -> Vec<TokenTree> {
    if let Some(TokenTree::Group(fields)) = item.pop() {
        let mut widened = Group::new(Delimiter::Brace, widen_fields(fields.stream()));
        widened.set_span(fields.span());
        widen(&mut item);
        item.push(TokenTree::Group(widened));
    }
    item
}

fn widen_fields(fields: TokenStream) -> TokenStream {
    let mut output: Vec<TokenTree> = Vec::new();
    let mut field: Vec<TokenTree> = Vec::new();
    // A comma inside `<..>` separates type arguments, not fields.
    let mut angles = 0usize;
    let mut arrow = false;
    for tree in fields {
        let mut ends = false;
        if let TokenTree::Punct(punct) = &tree {
            match punct.as_char() {
                '<' => angles += 1,
                // The `>` of `->` closes nothing.
                '>' if !arrow => angles = angles.saturating_sub(1),
                ',' if angles == 0 => ends = true,
                _ => {}
            }
            arrow = punct.as_char() == '-' && punct.spacing() == Spacing::Joint;
        } else {
            arrow = false;
        }
        field.push(tree);
        if ends {
            widen(&mut field);
            output.append(&mut field);
        }
    }
    widen(&mut field);
    output.append(&mut field);
    output.into_iter().collect()
}

/// Inserts `pub(super)` before the first identifier, which follows the attributes,
/// unless a visibility is already there.
fn widen(tokens: &mut Vec<TokenTree>) {
    let Some(first) = tokens
        .iter()
        .position(|tree| matches!(tree, TokenTree::Ident(_)))
    else {
        return;
    };
    if !matches!(&tokens[first], TokenTree::Ident(ident) if ident == "pub") {
        let scope = TokenStream::from(TokenTree::Ident(Ident::new("super", Span::call_site())));
        let visibility = [
            TokenTree::Ident(Ident::new("pub", Span::call_site())),
            TokenTree::Group(Group::new(Delimiter::Parenthesis, scope)),
        ];
        tokens.splice(first..first, visibility);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(tokens: TokenStream) -> String {
        tokens
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn spread_text(source: &str, parts: usize) -> String {
        text(spread(source.parse().unwrap(), parts).unwrap())
    }

    #[test]
    fn component_structs_move_and_everything_else_keeps_its_place() {
        let output = spread_text(
            "mod slint_generatedX { #![allow(dead_code)] use a::{b, c}; \
             #[derive(Default)] #[pin] pub struct InnerA { r#x: sp::Property<(i32, bool)>, } \
             impl InnerA { fn new() {} } \
             const _: () = { use q::*; V!(static VT for self::InnerB); }; \
             #[pin_drop] struct InnerB { #[skip] a: InnerA, pub b: Map<K, fn(u8, u8) -> V>, c: u8 } \
             pub struct r#Public { pub r#field: bool } \
             pub struct r#Handle(sp::VRc<V, InnerB>); \
             static BYTES: &'static [u8] = b\"{\"; } pub use slint_generatedX::{r#Handle};",
            2,
        );
        let expected = text(
            "mod slint_generatedX { #![allow(dead_code)] use a::{b, c}; \
             impl InnerA { fn new() {} } \
             const _: () = { use q::*; V!(static VT for self::InnerB); }; \
             pub struct r#Public { pub r#field: bool } \
             pub struct r#Handle(sp::VRc<V, InnerB>); \
             static BYTES: &'static [u8] = b\"{\"; \
             mod slint_part_0 { #[allow(unused_imports)] use super::*; \
             #[derive(Default)] #[pin] pub struct InnerA { pub(super) r#x: sp::Property<(i32, bool)>, } } \
             #[allow(unused_imports)] pub use slint_part_0::*; \
             mod slint_part_1 { #[allow(unused_imports)] use super::*; \
             #[pin_drop] pub(super) struct InnerB { #[skip] pub(super) a: InnerA, \
             pub b: Map<K, fn(u8, u8) -> V>, pub(super) c: u8 } } \
             #[allow(unused_imports)] pub use slint_part_1::*; } \
             pub use slint_generatedX::{r#Handle};"
                .parse()
                .unwrap(),
        );
        assert_eq!(output, expected);
    }

    #[test]
    fn fewer_components_than_parts_leave_no_empty_module() {
        let output = spread_text("mod m { struct InnerA { a: u8 } }", 32);
        assert_eq!(output.matches("mod slint_part_").count(), 1);
    }

    #[test]
    fn unknown_module_items_and_missing_modules_are_errors() {
        assert!(spread("mod m { generate!{} }".parse().unwrap(), 4).is_err());
        assert!(
            spread(
                "mod m { struct InnerA { a: u8 } const X: u8 = 1 }"
                    .parse()
                    .unwrap(),
                4
            )
            .is_err()
        );
        assert!(spread("pub use a::b;".parse().unwrap(), 4).is_err());
        let foreign_block = "mod m { extern \"C\" { } struct InnerA { a: u8 } }";
        assert!(spread(foreign_block.parse().unwrap(), 4).is_err());
        assert!(spread("mod m { pub struct r#A { pub a: u8 } }".parse().unwrap(), 4).is_err());
        // Paths and visibilities relative to the module would change their meaning.
        for field in [
            "a: super::T",
            "a: Map<u8, self::T>",
            "pub(self) a: u8",
            "pub(in super) a: u8",
        ] {
            let relative = format!("mod m {{ struct InnerA {{ {field} }} }}");
            assert!(spread(relative.parse().unwrap(), 4).is_err(), "{field}");
        }
    }
}
