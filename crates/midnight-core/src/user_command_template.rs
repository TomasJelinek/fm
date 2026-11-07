//! Literal native-data expansion. Never builds or quotes a shell command.
use std::ffi::{OsStr, OsString};

const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;
const MAX_TEMPLATE_TOKENS: usize = 4096;
const MAX_EXPANDED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UserCommandVariable {
    ActivePath,
    OtherPath,
    FilePath,
    FileName,
    FileStem,
    FileExtension,
    SelectedPaths,
}
impl UserCommandVariable {
    pub fn parse(name: &str) -> Result<Self, &'static str> {
        match name {
            "active.path" => Ok(Self::ActivePath),
            "other.path" => Ok(Self::OtherPath),
            "file.path" => Ok(Self::FilePath),
            "file.name" => Ok(Self::FileName),
            "file.stem" => Ok(Self::FileStem),
            "file.extension" => Ok(Self::FileExtension),
            "selected.paths" => Ok(Self::SelectedPaths),
            _ => Err("Unknown Custom Command template variable."),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::ActivePath => "active.path",
            Self::OtherPath => "other.path",
            Self::FilePath => "file.path",
            Self::FileName => "file.name",
            Self::FileStem => "file.stem",
            Self::FileExtension => "file.extension",
            Self::SelectedPaths => "selected.paths",
        }
    }
    pub fn environment_key(self) -> Option<&'static str> {
        match self {
            Self::ActivePath => Some("ME_ACTIVE_PATH"),
            Self::OtherPath => Some("ME_OTHER_PATH"),
            Self::FilePath => Some("ME_FILE_PATH"),
            Self::FileName => Some("ME_FILE_NAME"),
            Self::FileStem => Some("ME_FILE_STEM"),
            Self::FileExtension => Some("ME_FILE_EXTENSION"),
            Self::SelectedPaths => None,
        }
    }
}
enum Token<'a> {
    Literal(&'a str),
    Value(UserCommandVariable),
}
pub struct UserCommandTemplate<'a> {
    tokens: Vec<Token<'a>>,
    group: bool,
}

impl<'a> UserCommandTemplate<'a> {
    pub fn parse(source: &'a str) -> Result<Self, &'static str> {
        if source.len() > MAX_TEMPLATE_BYTES || source.contains('\0') {
            return Err("Template exceeds its 1 MiB input budget or contains NUL.");
        }
        let mut tokens = Vec::new();
        let mut cursor = 0;
        while cursor < source.len() {
            if tokens.len() >= MAX_TEMPLATE_TOKENS {
                return Err("Template exceeds its 4096-token resource budget.");
            }
            let Some(offset) = source[cursor..].find('$') else {
                tokens.push(Token::Literal(&source[cursor..]));
                break;
            };
            let dollar = cursor + offset;
            if dollar > cursor {
                tokens.push(Token::Literal(&source[cursor..dollar]));
            }
            let tail = &source[dollar..];
            if tail.starts_with("$$") {
                tokens.push(Token::Literal(&source[dollar..dollar + 1]));
                cursor = dollar + 2;
            } else if let Some(variable) = tail.strip_prefix("${") {
                let end = variable
                    .find('}')
                    .ok_or("Unterminated Custom Command template variable.")?;
                tokens.push(Token::Value(UserCommandVariable::parse(&variable[..end])?));
                cursor = dollar + 2 + end + 1;
            } else {
                tokens.push(Token::Literal(&source[dollar..dollar + 1]));
                cursor = dollar + 1;
            }
        }
        if tokens.len() > MAX_TEMPLATE_TOKENS {
            return Err("Template exceeds its 4096-token resource budget.");
        }
        let group = matches!(
            &tokens[..],
            [Token::Value(UserCommandVariable::SelectedPaths)]
        );
        if !group
            && tokens
                .iter()
                .any(|token| matches!(token, Token::Value(UserCommandVariable::SelectedPaths)))
        {
            return Err("selected.paths must occupy one complete Program argument.");
        }
        Ok(Self { tokens, group })
    }
    pub fn is_group(&self) -> bool {
        self.group
    }

    pub fn variables(&self) -> impl Iterator<Item = UserCommandVariable> + '_ {
        self.tokens.iter().filter_map(|token| match token {
            Token::Value(variable) => Some(*variable),
            Token::Literal(_) => None,
        })
    }

    pub fn expand<'v>(
        &self,
        mut value: impl FnMut(UserCommandVariable) -> Option<&'v OsStr>,
    ) -> Result<OsString, &'static str> {
        if self.group {
            return Err("selected.paths requires explicit argument-list expansion.");
        }
        let mut result = OsString::new();
        for token in &self.tokens {
            let part = match token {
                Token::Literal(text) => OsStr::new(text),
                Token::Value(variable) => {
                    value(*variable).ok_or("Required captured template value is unavailable.")?
                }
            };
            let raw = part.as_encoded_bytes();
            if raw.contains(&0) {
                return Err("Captured template values cannot contain NUL.");
            }
            if result
                .as_encoded_bytes()
                .len()
                .checked_add(raw.len())
                .is_none_or(|bytes| bytes > MAX_EXPANDED_BYTES)
            {
                return Err("Expanded argument exceeds the 8 MiB workload budget.");
            }
            result.push(part);
        }
        Ok(result)
    }
}
