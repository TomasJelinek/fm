//! Pure Make File List formatting; environment values and metadata come from backend snapshots.
use std::collections::{BTreeMap, BTreeSet};

/// Resource envelope for user-authored syntax, not a filesystem path limit.
pub const MAX_FILE_LIST_TEMPLATE_BYTES: usize = 64 * 1024;
/// A single expanded row is bounded; callers can stream arbitrarily many rows.
pub const MAX_FILE_LIST_ROW_BYTES: usize = 8 * 1024 * 1024;

/// Display values captured on a backend worker. Formatting never reads the OS.
#[derive(Clone, Debug, Default)]
pub struct FileListRecord {
    pub file_name: String,
    pub name_part: String,
    pub extension: String,
    pub size: String,
    pub date: String,
    pub time: String,
    pub attributes: String,
    pub drive: String,
    pub path: String,
    pub environment: BTreeMap<String, String>,
}

/// Salamander-compatible variables and widths over lossless Unicode strings.
///
/// `:N` deliberately truncates/pads to N Unicode scalar values (1..=9999), while
/// `:max` pads to the largest measured value. Neither mode splits UTF-8. Scalar
/// widths are not terminal cell widths; combining and wide glyphs remain intact.
#[derive(Clone, Debug)]
pub struct FileListTemplate {
    tokens: Vec<Token>,
    max_count: usize,
    environment: BTreeSet<String>,
}

#[derive(Clone, Debug)]
enum Token {
    Literal(String),
    Variable(Field, Width),
    Environment(String),
}

#[derive(Clone, Copy, Debug)]
enum Width {
    Natural,
    Fixed(usize),
    Maximum(usize),
}

#[derive(Clone, Copy, Debug)]
enum Field {
    FileName,
    NamePart,
    Extension,
    Size,
    Date,
    Time,
    Attributes,
    Drive,
    Path,
    Lf,
    Cr,
    Crlf,
    Tab,
}

impl Field {
    fn parse(name: &str) -> Option<Self> {
        [
            ("FileName", Self::FileName),
            ("FileNamePart", Self::NamePart),
            ("FileExtension", Self::Extension),
            ("FileSize", Self::Size),
            ("FileDate", Self::Date),
            ("FileTime", Self::Time),
            ("FileAttributes", Self::Attributes),
            ("FileDOSName", Self::FileName),
            ("Drive", Self::Drive),
            ("Path", Self::Path),
            ("DOSPath", Self::Path),
            ("LF", Self::Lf),
            ("CR", Self::Cr),
            ("CRLF", Self::Crlf),
            ("TAB", Self::Tab),
        ]
        .into_iter()
        .find_map(|(candidate, field)| name.eq_ignore_ascii_case(candidate).then_some(field))
    }

    fn value(self, record: &FileListRecord) -> &str {
        match self {
            Self::FileName => &record.file_name,
            Self::NamePart => &record.name_part,
            Self::Extension => &record.extension,
            Self::Size => &record.size,
            Self::Date => &record.date,
            Self::Time => &record.time,
            Self::Attributes => &record.attributes,
            Self::Drive => &record.drive,
            Self::Path => &record.path,
            Self::Lf => "\n",
            Self::Cr => "\r",
            Self::Crlf => "\r\n",
            Self::Tab => "\t",
        }
    }
}

impl FileListTemplate {
    /// Validate syntax without resolving metadata or reading environment variables.
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.len() > MAX_FILE_LIST_TEMPLATE_BYTES {
            return Err("The line template exceeds the 64 KiB syntax budget.".into());
        }
        if source.contains('\0') {
            return Err("A file-list template cannot contain NUL.".into());
        }
        let mut result = Self {
            tokens: Vec::new(),
            max_count: 0,
            environment: BTreeSet::new(),
        };
        let mut rest = source;
        while let Some(dollar) = rest.find('$') {
            result.literal(&rest[..dollar]);
            rest = &rest[dollar + 1..];
            if let Some(tail) = rest.strip_prefix('$') {
                result.literal("$");
                rest = tail;
                continue;
            }
            let (close, environment) = match rest.as_bytes().first() {
                Some(b'(') => (')', false),
                Some(b'[') => (']', true),
                None => return Err("A trailing dollar sign must be written as $$.".into()),
                _ => return Err("After $ use (, [ or another $ for a literal dollar sign.".into()),
            };
            let end = rest
                .find(close)
                .ok_or_else(|| format!("The line template is missing a closing {close}."))?;
            let name = &rest[1..end];
            if environment {
                if name.is_empty() || name.contains('=') || name.chars().any(char::is_control) {
                    return Err("An environment variable needs a valid nonempty name.".into());
                }
                result.environment.insert(name.to_owned());
                result.tokens.push(Token::Environment(name.to_owned()));
            } else {
                let (name, width) = match name.split_once(':') {
                    Some((name, suffix)) => (name, result.width(suffix)?),
                    None => (name, Width::Natural),
                };
                let field = Field::parse(name)
                    .ok_or_else(|| format!("Unknown file-list variable: {name}."))?;
                result.tokens.push(Token::Variable(field, width));
            }
            rest = &rest[end + 1..];
        }
        result.literal(rest);
        Ok(result)
    }

    /// Measure `:max` fields in a first pass over the exact captured row set.
    /// The vector belongs to this template and must start empty or have its size.
    pub fn measure(&self, record: &FileListRecord, widths: &mut Vec<usize>) -> Result<(), String> {
        if widths.is_empty() {
            widths.resize(self.max_count, 0);
        }
        self.check_widths(widths)?;
        for token in &self.tokens {
            let value = token.value(record)?;
            validate_value(value)?;
            if let Token::Variable(_, Width::Maximum(index)) = token {
                if value.len() > MAX_FILE_LIST_ROW_BYTES {
                    return Err(row_budget());
                }
                widths[*index] = widths[*index].max(value.chars().count());
            }
        }
        Ok(())
    }

    /// Expand one row after the optional `:max` measurement pass.
    /// Environment substitutions are literal and never recursively evaluated.
    pub fn render(&self, record: &FileListRecord, widths: &[usize]) -> Result<String, String> {
        self.check_widths(widths)?;
        let mut output = String::new();
        for token in &self.tokens {
            let value = token.value(record)?;
            validate_value(value)?;
            let (value, padding) = match token {
                Token::Variable(_, Width::Fixed(width)) => fixed(value, *width),
                Token::Variable(_, Width::Maximum(index)) => {
                    let length = value.chars().count();
                    let padding = widths[*index].checked_sub(length).ok_or_else(|| {
                        "Measure :max widths for all captured rows before rendering.".to_owned()
                    })?;
                    (value, padding)
                }
                _ => (value, 0),
            };
            append(&mut output, value, padding)?;
        }
        Ok(output)
    }

    /// Unique referenced names, in sorted order. The backend resolves them once
    /// using host environment rules, preserving these keys in each record.
    pub fn environment_names(&self) -> impl Iterator<Item = &str> {
        self.environment.iter().map(String::as_str)
    }

    fn literal(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        if let Some(Token::Literal(previous)) = self.tokens.last_mut() {
            previous.push_str(value);
        } else {
            self.tokens.push(Token::Literal(value.to_owned()));
        }
    }

    fn width(&mut self, source: &str) -> Result<Width, String> {
        if source.eq_ignore_ascii_case("max") {
            let index = self.max_count;
            self.max_count += 1;
            return Ok(Width::Maximum(index));
        }
        if !source.is_empty()
            && source.len() <= 4
            && source.bytes().all(|c| c.is_ascii_digit())
            && let Ok(width @ 1..=9999) = source.parse::<usize>()
        {
            return Ok(Width::Fixed(width));
        }
        Err("A variable width must be 1 through 9999 or max.".into())
    }

    fn check_widths(&self, widths: &[usize]) -> Result<(), String> {
        if widths.len() != self.max_count {
            return Err("The measured widths do not match this file-list template.".into());
        }
        if widths.iter().any(|&width| width > MAX_FILE_LIST_ROW_BYTES) {
            return Err(row_budget());
        }
        Ok(())
    }
}

impl Token {
    fn value<'a>(&'a self, record: &'a FileListRecord) -> Result<&'a str, String> {
        match self {
            Self::Literal(value) => Ok(value),
            Self::Variable(field, _) => Ok(field.value(record)),
            Self::Environment(name) => record
                .environment
                .get(name)
                .map(String::as_str)
                .ok_or_else(|| format!("Environment variable is unavailable: {name}.")),
        }
    }
}

fn validate_value(value: &str) -> Result<(), String> {
    if value.contains('\0') {
        Err("A file-list value cannot contain NUL.".into())
    } else {
        Ok(())
    }
}

fn fixed(value: &str, width: usize) -> (&str, usize) {
    let mut count = 0;
    for (index, (position, _)) in value.char_indices().enumerate() {
        if index == width {
            return (&value[..position], 0);
        }
        count = index + 1;
    }
    (value, width - count)
}

fn append(output: &mut String, value: &str, padding: usize) -> Result<(), String> {
    let additional = value.len().checked_add(padding).ok_or_else(row_budget)?;
    if output
        .len()
        .checked_add(additional)
        .is_none_or(|length| length > MAX_FILE_LIST_ROW_BYTES)
    {
        return Err(row_budget());
    }
    output
        .try_reserve(additional)
        .map_err(|_| "Not enough memory to format a file-list row.".to_owned())?;
    output.push_str(value);
    output.extend(std::iter::repeat_n(' ', padding));
    Ok(())
}

fn row_budget() -> String {
    "The expanded file-list row exceeds the 8 MiB resource budget.".into()
}

#[cfg(test)]
#[path = "file_list_format_tests.rs"]
mod tests;
