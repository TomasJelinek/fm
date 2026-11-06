use crate::FsError;

/// Immutable number preferences read on a worker. Formatting this snapshot does no OS I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberLocale {
    pub decimal_separator: String,
    pub group_separator: String,
    /// Group widths from the least significant digits towards the most significant.
    pub grouping: Vec<u8>,
    pub repeat_last_group: bool,
}

impl Default for NumberLocale {
    fn default() -> Self {
        Self {
            decimal_separator: ".".into(),
            group_separator: String::new(),
            grouping: Vec::new(),
            repeat_last_group: false,
        }
    }
}

impl NumberLocale {
    /// Formats the complete integer without floating-point conversion or precision loss.
    pub fn format_u64(&self, value: u64) -> String {
        let digits = value.to_string();
        if self.group_separator.is_empty() || self.grouping.first().is_none_or(|size| *size == 0) {
            return digits;
        }
        let mut pieces = Vec::new();
        let mut end = digits.len();
        let mut group = 0usize;
        while end > 0 {
            let width = self.grouping.get(group).copied().or_else(|| {
                self.repeat_last_group
                    .then(|| self.grouping.last().copied())
                    .flatten()
            });
            let Some(width) = width.filter(|width| *width > 0) else {
                pieces.push(&digits[..end]);
                break;
            };
            let start = end.saturating_sub(usize::from(width));
            pieces.push(&digits[start..end]);
            end = start;
            group += 1;
        }
        pieces.reverse();
        pieces.join(&self.group_separator)
    }
}

/// Date and time stay separate because either localized field can contain spaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalizedDateTime {
    pub date: String,
    pub time: String,
}

/// Optional native locale discovery. Call only on a startup/backend worker.
pub trait LocaleService: Send + Sync {
    fn number_locale(&self) -> Result<NumberLocale, FsError>;
}

#[cfg(test)]
#[path = "localization_tests.rs"]
mod tests;
