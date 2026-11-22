//! GitHub-style heading slugs, which `#fragment` links in Markdown name: the heading's
//! text in lowercase with punctuation and symbols removed and each space a hyphen, and a
//! repeated slug numbered `-1`, `-2`, … in document order (as github-slugger does).
use std::collections::HashMap;

#[cfg(test)]
#[path = "link_slug_tests.rs"]
mod tests;

/// The slugs of one document's headings, in document order.
#[derive(Default)]
pub(crate) struct Slugger {
    /// Each slug given so far, with how many later headings repeated it.
    seen: HashMap<String, usize>,
}

impl Slugger {
    /// The slug of the next heading, whose displayed text is `text`.
    pub(crate) fn slug(&mut self, text: &str) -> String {
        let base = slug(text);
        let mut unique = base.clone();
        while self.seen.contains_key(&unique) {
            let count = self.seen.entry(base.clone()).or_insert(0);
            *count += 1;
            unique = format!("{base}-{count}");
        }
        self.seen.insert(unique.clone(), 0);
        unique
    }

    /// Whether the next heading, `text`, is the one `fragment` names. GitHub's anchors are
    /// lowercase; a fragment written with capitals matches too.
    pub(crate) fn matches(&mut self, text: &str, fragment: &str) -> bool {
        let slug = self.slug(text);
        slug == fragment || slug == fragment.to_lowercase()
    }
}

/// One heading's slug before repeats are numbered.
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| kept(*c))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// Letters, marks, numbers, connector punctuation, space and hyphen stay.
fn kept(c: char) -> bool {
    c == ' '
        || c == '-'
        || c.is_alphanumeric()
        || matches!(
            c,
            '_' | '\u{203f}'
                | '\u{2040}'
                | '\u{2054}'
                | '\u{fe33}'
                | '\u{fe34}'
                | '\u{fe4d}'
                | '\u{fe4e}'
                | '\u{fe4f}'
                | '\u{ff3f}'
        )
        || matches!(
            c,
            '\u{0300}'..='\u{036f}'
                | '\u{1ab0}'..='\u{1aff}'
                | '\u{1dc0}'..='\u{1dff}'
                | '\u{20d0}'..='\u{20ff}'
                | '\u{fe20}'..='\u{fe2f}'
        )
}
