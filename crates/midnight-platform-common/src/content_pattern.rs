//! Bounded regular expressions for Find's line-based content search. Patterns are
//! untrusted input: compiled programs and lazy DFAs are capped, nesting is
//! limited, and matching runs in time linear in the searched line.
use regex::{Regex, RegexBuilder};

/// Compiled program and lazy DFA ceiling of each pattern (the panel filter's budget).
const ENGINE_BYTES: usize = crate::filter::REGEX_BYTES;
const NEST_LIMIT: u32 = 64;

/// A content pattern that matches within one line of decoded text.
pub struct LinePattern {
    regex: Regex,
}

impl LinePattern {
    /// `whole_words` requires a word boundary on both sides of every match.
    /// Errors are the regular expression library's own explanation.
    pub fn new(pattern: &str, case_sensitive: bool, whole_words: bool) -> Result<Self, String> {
        let build = |source: &str| {
            RegexBuilder::new(source)
                .case_insensitive(!case_sensitive)
                .size_limit(ENGINE_BYTES)
                .dfa_size_limit(ENGINE_BYTES)
                .nest_limit(NEST_LIMIT)
                .build()
                .map_err(|error| error.to_string())
        };
        // The user's pattern is validated alone first: wrapping an invalid
        // pattern in a group could turn it into a different valid one.
        let regex = build(pattern)?;
        let regex = if whole_words {
            build(&format!(r"\b(?:{pattern})\b"))?
        } else {
            regex
        };
        Ok(Self { regex })
    }

    pub fn is_match(&self, line: &str) -> bool {
        self.regex.is_match(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_whole_words_and_errors() {
        let pattern = LinePattern::new(r"qu?ick\d+", false, false).unwrap();
        assert!(pattern.is_match("a QUICK42 fox"));
        assert!(!pattern.is_match("a quick fox"));
        let exact = LinePattern::new("Quick", true, false).unwrap();
        assert!(!exact.is_match("quick"));
        let words = LinePattern::new("quick|lazy", false, true).unwrap();
        assert!(words.is_match("the lazy dog"));
        assert!(!words.is_match("quickly lazyness"));
        let error = LinePattern::new("a)|(b", false, true).err().unwrap();
        assert!(
            error.contains("unopened group") || error.contains("regex parse error"),
            "{error}"
        );
        let nested = format!("{}a{}", "(".repeat(70), ")".repeat(70));
        assert!(
            LinePattern::new(&nested, false, false).is_err(),
            "nesting is bounded"
        );
    }
}
