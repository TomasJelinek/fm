//! Drops the match arms that the generator repeats in the accessibility lookups.
//!
//! Where an element sets an accessible role or action on an instance of a
//! sub-component, slint's generator writes two arms with the same item index into
//! `accessible_role` and `supported_accessibility_actions`: the element's own value
//! and then the sub-component's. Only the first can match. rustc examines every
//! unreachable arm against the source text before it consults the lint level, which
//! the generated code allows, and in a module of this size that took about 9 ms per
//! arm: 5 s of every compilation of this crate. The machine code does not change.
//! Measurements: docs/verification/2026-11-19-slint-build-speed.md.
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use std::collections::HashSet;

/// Generated functions whose `match index` repeats literal arms.
const LOOKUPS: [&str; 2] = ["accessible_role", "supported_accessibility_actions"];

/// The generated tokens without the repeated arms of the accessibility lookups in
/// the generated module's `impl` blocks. The search for them is loose, because such an
/// arm is unreachable wherever it stands; a match with an arm this pass does not
/// recognise stays as it is, since a repeated arm only costs compile time.
pub fn prune(tokens: TokenStream) -> TokenStream {
    blocks(tokens, starts_module, &|module| {
        blocks(module, starts_impl, &|implementation| {
            blocks(implementation, starts_lookup, &|body| {
                blocks(body, starts_match, &arms)
            })
        })
    })
}

/// `stream` with `inner` applied to the brace group that follows each place where
/// `starts` holds for a token and the one before it. A `;` in between means a
/// declaration without a body.
fn blocks(
    stream: TokenStream,
    starts: fn(Option<&TokenTree>, &TokenTree) -> bool,
    inner: &dyn Fn(TokenStream) -> TokenStream,
) -> TokenStream {
    let mut output: Vec<TokenTree> = Vec::new();
    let mut expected = false;
    for tree in stream {
        let tree = match tree {
            TokenTree::Group(group) if expected && group.delimiter() == Delimiter::Brace => {
                expected = false;
                let mut rewritten = Group::new(Delimiter::Brace, inner(group.stream()));
                rewritten.set_span(group.span());
                TokenTree::Group(rewritten)
            }
            tree => {
                if starts(output.last(), &tree) {
                    expected = true;
                } else if is_punct(&tree, ';') {
                    expected = false;
                }
                tree
            }
        };
        output.push(tree);
    }
    output.into_iter().collect()
}

fn starts_module(previous: Option<&TokenTree>, tree: &TokenTree) -> bool {
    is_ident(previous, "mod") && matches!(tree, TokenTree::Ident(_))
}

fn starts_impl(_: Option<&TokenTree>, tree: &TokenTree) -> bool {
    is_ident(Some(tree), "impl")
}

fn starts_lookup(previous: Option<&TokenTree>, tree: &TokenTree) -> bool {
    is_ident(previous, "fn") && LOOKUPS.iter().any(|name| is_ident(Some(tree), name))
}

fn starts_match(previous: Option<&TokenTree>, tree: &TokenTree) -> bool {
    is_ident(previous, "match") && is_ident(Some(tree), "index")
}

fn is_ident(tree: Option<&TokenTree>, name: &str) -> bool {
    matches!(tree, Some(TokenTree::Ident(ident)) if ident == name)
}

fn is_punct(tree: &TokenTree, character: char) -> bool {
    matches!(tree, TokenTree::Punct(punct) if punct.as_char() == character)
}

/// The arms of a `match` without every arm whose whole pattern is a literal that an
/// earlier arm already matches without a guard. The arms are returned unchanged
/// unless each of them is `literal =>`, `literal ..= literal =>` or `_ =>`.
fn arms(stream: TokenStream) -> TokenStream {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    let Some(bounds) = arm_bounds(&trees) else {
        return trees.into_iter().collect();
    };
    let mut matched = HashSet::new();
    let mut repeated = vec![false; trees.len()];
    for (start, end) in bounds {
        // Only the equal literal is proved unreachable here. An arm that a range or
        // `_` before it covers stays; rustc discards it.
        if let (TokenTree::Literal(literal), true) = (&trees[start], arrow_at(&trees, start + 1))
            && !matched.insert(literal.to_string())
        {
            repeated[start..end].fill(true);
        }
    }
    trees
        .into_iter()
        .zip(repeated)
        .filter_map(|(tree, repeated)| (!repeated).then_some(tree))
        .collect()
}

/// Where each arm starts and ends, its separating comma included, or `None` if the
/// arms are not all of the forms this pass knows. Every arm must hold exactly one
/// `=>` outside nested groups, so an arm of another form is never taken for part of
/// its neighbour.
fn arm_bounds(trees: &[TokenTree]) -> Option<Vec<(usize, usize)>> {
    let mut bounds = Vec::new();
    let mut start = 0;
    while start < trees.len() {
        let pattern = pattern_length(trees, start)?;
        let mut end = start + pattern + 2;
        while end < trees.len() {
            if arrow_at(trees, end) {
                return None;
            }
            end += 1;
            if is_punct(&trees[end - 1], ',') && pattern_length(trees, end).is_some() {
                break;
            }
        }
        bounds.push((start, end));
        start = end;
    }
    Some(bounds)
}

/// Number of tokens of the arm pattern that starts at `at` and is followed by `=>`.
fn pattern_length(trees: &[TokenTree], at: usize) -> Option<usize> {
    let literal = |at: usize| matches!(trees.get(at), Some(TokenTree::Literal(_)));
    let punct = |at: usize, character| trees.get(at).is_some_and(|tree| is_punct(tree, character));
    let length = if is_ident(trees.get(at), "_") || (literal(at) && arrow_at(trees, at + 1)) {
        1
    } else if literal(at)
        && punct(at + 1, '.')
        && punct(at + 2, '.')
        && punct(at + 3, '=')
        && literal(at + 4)
    {
        5
    } else {
        return None;
    };
    arrow_at(trees, at + length).then_some(length)
}

/// Whether the tokens at `at` are `=>`.
fn arrow_at(trees: &[TokenTree], at: usize) -> bool {
    matches!(
        (trees.get(at), trees.get(at + 1)),
        (Some(TokenTree::Punct(equals)), Some(TokenTree::Punct(greater)))
            if equals.as_char() == '='
                && equals.spacing() == proc_macro2::Spacing::Joint
                && greater.as_char() == '>'
    )
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

    fn pruned(source: &str) -> String {
        text(prune(source.parse().unwrap()))
    }

    fn unchanged(source: &str) {
        assert_eq!(pruned(source), text(source.parse().unwrap()));
    }

    /// The generated module with one lookup that has these arms.
    fn lookup(name: &str, arms: &str) -> String {
        format!(
            "mod slint_generatedX {{ impl InnerA {{ \
             fn {name}(self: Pin<&Self>, index: u32) -> sp::Role {{ \
             #![allow(unused)] let _self = self; match index {{ {arms} }} }} }} }}"
        )
    }

    #[test]
    fn a_literal_arm_repeated_after_an_equal_one_is_dropped() {
        let arms = "3u32 => sp::Role::Button, 7u32 => sp::Role::Text, \
                    3u32 => F.r#close().apply_pin(_self).accessible_role(0), \
                    9u32 ..= 11u32 => F.r#close().apply_pin(_self).accessible_role(index - 9u32 + 1), \
                    7u32 => g::<A, B>(index), _ => sp::Role::default(),";
        let kept = "3u32 => sp::Role::Button, 7u32 => sp::Role::Text, \
                    9u32 ..= 11u32 => F.r#close().apply_pin(_self).accessible_role(index - 9u32 + 1), \
                    _ => sp::Role::default(),";
        for name in LOOKUPS {
            assert_eq!(
                pruned(&lookup(name, arms)),
                text(lookup(name, kept).parse().unwrap())
            );
        }
    }

    #[test]
    fn the_last_arm_may_be_the_repeated_one_and_need_no_comma() {
        let source = lookup("accessible_role", "1u32 => a, 2u32 => { b }, 1u32 => c");
        let expected = lookup("accessible_role", "1u32 => a, 2u32 => { b },");
        assert_eq!(pruned(&source), text(expected.parse().unwrap()));
    }

    #[test]
    fn arms_this_pass_does_not_prove_unreachable_stay() {
        // Different spelling, a range before the literal, a literal after `_`.
        unchanged(&lookup(
            "accessible_role",
            "1u32 => a, 1 => b, 0x1u32 => c, _ => d",
        ));
        unchanged(&lookup(
            "accessible_role",
            "1u32 ..= 4u32 => a, 2u32 => b, _ => c",
        ));
        unchanged(&lookup("accessible_role", "_ => a, 2u32 => b"));
    }

    #[test]
    fn a_match_with_an_arm_of_another_form_is_left_alone() {
        unchanged(&lookup(
            "accessible_role",
            "1u32 => a, 1u32 if c => b, 1u32 => d, _ => e",
        ));
        unchanged(&lookup(
            "accessible_role",
            "1u32 => a, 2u32 | 1u32 => b, 1u32 => d",
        ));
        unchanged(&lookup("accessible_role", "1u32 => a, X => b, 1u32 => c"));
        unchanged(&lookup(
            "accessible_role",
            "x @ 1u32 => a, 1u32 => b, 1u32 => c",
        ));
        // A block arm without a comma is valid Rust that the generator does not write.
        unchanged(&lookup(
            "accessible_role",
            "1u32 => { a } 1u32 => b, 1u32 => c",
        ));
        unchanged(&lookup("accessible_role", ""));
    }

    #[test]
    fn other_functions_matches_and_places_are_not_touched() {
        let arms = "1u32 => a, 1u32 => b, _ => c";
        unchanged(&lookup("item_geometry", arms));
        unchanged(&lookup("accessible_role", arms).replace("match index", "match other"));
        unchanged(&format!(
            "mod m {{ fn accessible_role(index: u32) -> R {{ match index {{ {arms} }} }} }}"
        ));
        unchanged(&format!(
            "impl A {{ fn accessible_role(index: u32) -> R {{ match index {{ {arms} }} }} }}"
        ));
        unchanged(
            "mod m { impl T for A { fn accessible_role(index: u32) -> R; } struct B { a: u8 } }",
        );
    }
}
