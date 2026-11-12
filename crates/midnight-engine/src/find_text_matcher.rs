//! Compiled literal matcher; only query-sized state survives content chunks.
use midnight_core::FindContentQuery;
use unicase::UniCase;
use unicode_segmentation::UnicodeSegmentation;

const SCALAR_FOLD_CACHE_ENTRIES: usize = 512;
const INLINE_FOLD_BYTES: usize = 12;

#[derive(Clone, Copy)]
struct ScalarFold {
    scalar: char,
    bytes: [u8; INLINE_FOLD_BYTES],
    length: u8,
}

enum FoldedScalar {
    Inline(ScalarFold),
    // Preserve exact library behavior if a future Unicode table exceeds our cache entry.
    Uncached(String),
}

impl FoldedScalar {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Inline(value) => &value.bytes[..usize::from(value.length)],
            Self::Uncached(value) => value.as_bytes(),
        }
    }
}

pub(super) struct Matcher {
    pattern: Vec<u8>,
    prefix: Vec<usize>,
    starts: Vec<bool>,
    cursor: usize,
    matched: usize,
    found: bool,
    pending_end: bool,
    previous_word: bool,
    case_sensitive: bool,
    whole_words: bool,
    // Allocate only for Unicode whole-word searches. Reset keeps these bounded,
    // query-independent folds so repeated text/files do not allocate per scalar.
    scalar_folds: Option<Box<[Option<ScalarFold>; SCALAR_FOLD_CACHE_ENTRIES]>>,
}

impl Matcher {
    pub(super) fn new(query: &FindContentQuery) -> Self {
        let pattern = if query.case_sensitive {
            query.text.as_bytes().to_vec()
        } else {
            UniCase::unicode(&query.text).to_folded_case().into_bytes()
        };
        let mut prefix = vec![0; pattern.len()];
        let mut matched = 0;
        for index in 1..pattern.len() {
            while matched != 0 && pattern[index] != pattern[matched] {
                matched = prefix[matched - 1];
            }
            if pattern[index] == pattern[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self {
            starts: vec![false; pattern.len()],
            pattern,
            prefix,
            cursor: 0,
            matched: 0,
            found: false,
            pending_end: false,
            previous_word: false,
            case_sensitive: query.case_sensitive,
            whole_words: query.whole_words,
            scalar_folds: None,
        }
    }

    pub(super) fn reset(&mut self) {
        self.cursor = 0;
        self.matched = 0;
        self.found = false;
        self.pending_end = false;
        self.previous_word = false;
        self.starts.fill(false);
    }

    pub(super) fn consume(&mut self, text: &str) {
        if self.found {
            return;
        }
        if !self.whole_words {
            // ASCII is common in logs/source. Fold bytes without another string.
            if self.case_sensitive || text.is_ascii() {
                for mut byte in text.bytes() {
                    if !self.case_sensitive {
                        byte = byte.to_ascii_lowercase();
                    }
                    if self.byte(byte) {
                        self.found = true;
                        break;
                    }
                }
            } else {
                let folded = UniCase::unicode(text).to_folded_case();
                for byte in folded.bytes() {
                    if self.byte(byte) {
                        self.found = true;
                        break;
                    }
                }
            }
            return;
        }
        for scalar in text.chars() {
            let word = word_character(scalar);
            if self.pending_end && !word {
                self.found = true;
                return;
            }
            self.pending_end = false;
            let mut encoded = [0; 4];
            let text: &str = scalar.encode_utf8(&mut encoded);
            let folded = (!self.case_sensitive && !scalar.is_ascii())
                .then(|| self.fold_scalar(scalar, text));
            let bytes = folded.as_ref().map_or(text.as_bytes(), FoldedScalar::bytes);
            for (index, &byte) in bytes.iter().enumerate() {
                self.starts[self.cursor] = index == 0 && !self.previous_word;
                self.cursor = (self.cursor + 1) % self.pattern.len();
                let byte = if self.case_sensitive {
                    byte
                } else {
                    byte.to_ascii_lowercase()
                };
                if self.byte(byte) && self.starts[self.cursor] && index + 1 == bytes.len() {
                    self.pending_end = true;
                }
            }
            self.previous_word = word;
        }
    }

    fn fold_scalar(&mut self, scalar: char, text: &str) -> FoldedScalar {
        let cache = self
            .scalar_folds
            .get_or_insert_with(|| Box::new([None; SCALAR_FOLD_CACHE_ENTRIES]));
        let index = scalar as usize % SCALAR_FOLD_CACHE_ENTRIES;
        if let Some(value) = cache[index]
            && value.scalar == scalar
        {
            return FoldedScalar::Inline(value);
        }
        let folded = UniCase::unicode(text).to_folded_case();
        if folded.len() > INLINE_FOLD_BYTES {
            return FoldedScalar::Uncached(folded);
        }
        let mut value = ScalarFold {
            scalar,
            bytes: [0; INLINE_FOLD_BYTES],
            length: folded.len() as u8,
        };
        value.bytes[..folded.len()].copy_from_slice(folded.as_bytes());
        cache[index] = Some(value);
        FoldedScalar::Inline(value)
    }

    fn byte(&mut self, byte: u8) -> bool {
        while self.matched != 0 && byte != self.pattern[self.matched] {
            self.matched = self.prefix[self.matched - 1];
        }
        if byte == self.pattern[self.matched] {
            self.matched += 1;
        }
        if self.matched == self.pattern.len() {
            self.matched = self.prefix[self.matched - 1];
            true
        } else {
            false
        }
    }

    pub(super) fn finish(&self) -> bool {
        self.found || self.pending_end
    }
}

fn word_character(value: char) -> bool {
    if value.is_ascii() {
        return value.is_ascii_alphanumeric() || value == '_';
    }
    if value.is_alphanumeric() {
        return true;
    }
    // Reuse pinned Unicode tables for extending marks/connectors without allocating.
    let mut bytes = [0; 5];
    bytes[0] = b'a';
    let length = 1 + value.encode_utf8(&mut bytes[1..]).len();
    let Ok(text) = std::str::from_utf8(&bytes[..length]) else {
        return false;
    };
    text.unicode_words()
        .next()
        .is_some_and(|word| word.len() == text.len())
}
