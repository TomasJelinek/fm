//! Host-owned comparator configuration. Core validates data; workers compile filename patterns.
use crate::{FilterMode, FsError, PanelFilter, UserCommandProcess, UserCommandProcessInvocation};
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

mod policy;
pub use policy::*;
mod feedback;
pub use feedback::*;
mod request;
pub use request::*;

pub const MAX_FILE_COMPARATOR_RULES: usize = 64;
pub const MAX_FILE_COMPARATOR_ARGUMENTS: usize = 64;
pub const MAX_FILE_COMPARATOR_ARGUMENT_BYTES: usize = 32 * 1024;
pub const MAX_FILE_COMPARATOR_MATCHER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileComparatorPreferences {
    pub rules: Vec<FileComparatorRule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileComparatorRule {
    pub enabled: bool,
    pub mode: FilterMode,
    pub expression: String,
    pub case_sensitive: bool,
    pub target: FileComparatorTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileComparatorTarget {
    Automatic,
    BuiltInText,
    BuiltInImages,
    External(FileComparatorProgram),
}

#[derive(Clone, PartialEq, Eq)]
pub struct FileComparatorProgram {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

impl fmt::Debug for FileComparatorProgram {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("FileComparatorProgram")
            .field("argument_count", &self.arguments.len())
            .field(
                "explicit_working_directory",
                &self.working_directory.is_some(),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileComparatorField {
    Rules,
    PatternMode,
    Expression,
    Executable,
    Arguments,
    WorkingDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileComparatorDiagnostic {
    /// Zero-based index in the submitted revision's ordered rules; None means the collection.
    pub rule: Option<usize>,
    pub field: FileComparatorField,
    pub message: String,
}

impl fmt::Display for FileComparatorDiagnostic {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(rule) = self.rule {
            write!(output, "Comparator rule {}: ", rule.saturating_add(1))?;
        }
        output.write_str(&self.message)
    }
}

impl std::error::Error for FileComparatorDiagnostic {}

impl From<FileComparatorDiagnostic> for FsError {
    fn from(error: FileComparatorDiagnostic) -> Self {
        Self::from_io(
            "validate file comparators",
            Path::new(""),
            std::io::Error::new(std::io::ErrorKind::InvalidInput, error),
        )
    }
}

impl FileComparatorPreferences {
    /// Bounded raw validation only, including disabled rules. No regex compilation or I/O.
    pub fn validate(&self) -> Result<(), FileComparatorDiagnostic> {
        if self.rules.len() > MAX_FILE_COMPARATOR_RULES {
            return Err(diagnostic(
                FileComparatorField::Rules,
                "At most 64 comparator rules are supported.",
            ));
        }
        for (index, rule) in self.rules.iter().enumerate() {
            rule.validate().map_err(|mut error| {
                error.rule = Some(index);
                error
            })?;
        }
        Ok(())
    }
}

impl FileComparatorRule {
    /// Disabled rows still compile on the backend before settings acceptance.
    pub fn filter(&self) -> PanelFilter {
        PanelFilter {
            enabled: true,
            mode: self.mode,
            expression: self.expression.clone(),
            case_sensitive: self.case_sensitive,
        }
    }

    pub fn validate(&self) -> Result<(), FileComparatorDiagnostic> {
        if self.mode == FilterMode::Expression {
            return Err(diagnostic(
                FileComparatorField::PatternMode,
                "Comparator rules support wildcards or regular expressions.",
            ));
        }
        if self.expression.trim().is_empty()
            || self.expression.len() > crate::MAX_FILTER_BYTES
            || self.expression.contains('\0')
        {
            return Err(diagnostic(
                FileComparatorField::Expression,
                "Enter a nonempty filename pattern of at most 4096 UTF-8 bytes without NUL.",
            ));
        }
        if let FileComparatorTarget::External(program) = &self.target {
            program
                .validate()
                .map_err(|(field, message)| diagnostic(field, message))?;
        }
        Ok(())
    }
}

impl FileComparatorProgram {
    fn validate(&self) -> Result<(), (FileComparatorField, &'static str)> {
        if !native_absolute(&self.executable) {
            return Err((
                FileComparatorField::Executable,
                "Comparator executable must be an absolute native path without NUL.",
            ));
        }
        if self
            .working_directory
            .as_ref()
            .is_some_and(|path| !native_absolute(path))
        {
            return Err((
                FileComparatorField::WorkingDirectory,
                "Comparator working directory must be an absolute native path without NUL.",
            ));
        }
        let bytes = self
            .arguments
            .iter()
            .try_fold(0usize, |sum, argument| sum.checked_add(argument.len()));
        if self.arguments.len() > MAX_FILE_COMPARATOR_ARGUMENTS
            || bytes.is_none_or(|bytes| bytes > MAX_FILE_COMPARATOR_ARGUMENT_BYTES)
        {
            return Err((
                FileComparatorField::Arguments,
                "Comparator arguments exceed the 64 argument or 32 KiB limit.",
            ));
        }
        if self
            .arguments
            .iter()
            .any(|argument| argument.contains('\0'))
        {
            return Err((
                FileComparatorField::Arguments,
                "Comparator arguments cannot contain NUL.",
            ));
        }
        if !self.arguments.iter().any(|argument| argument == "{left}")
            || !self.arguments.iter().any(|argument| argument == "{right}")
        {
            return Err((
                FileComparatorField::Arguments,
                "Comparator arguments must contain both {left} and {right} as complete arguments.",
            ));
        }
        Ok(())
    }

    /// Expand only complete placeholder items; preserve every other item and native path.
    pub fn arguments_for(&self, paths: &[PathBuf; 2]) -> Result<Vec<OsString>, &'static str> {
        self.validate().map_err(|(_, message)| message)?;
        if paths.iter().any(|path| !native_absolute(path)) {
            return Err("Comparison requires absolute local paths without NUL.");
        }
        Ok(self
            .arguments
            .iter()
            .map(|argument| match argument.as_str() {
                "{left}" => paths[0].as_os_str().to_owned(),
                "{right}" => paths[1].as_os_str().to_owned(),
                _ => OsString::from(argument),
            })
            .collect())
    }

    /// Construct a Program-only plan. Native adapters check executable availability and OS limits.
    pub fn process_for(
        &self,
        paths: &[PathBuf; 2],
    ) -> Result<ExternalCompareProcess, &'static str> {
        self.validate().map_err(|(_, message)| message)?;
        if paths.iter().any(|path| !native_absolute(path)) {
            return Err("Comparison requires absolute local paths without NUL.");
        }
        let working_directory = self
            .working_directory
            .as_deref()
            .or_else(|| self.executable.parent())
            .ok_or("Comparator executable has no parent directory.")?
            .to_owned();
        // Bound expansion BEFORE cloning repeated native paths. The existing process validator
        // remains authoritative for the actual owned plan, including Vec capacity accounting.
        let mut bytes = std::mem::size_of::<UserCommandProcess>()
            .checked_add(working_directory.as_os_str().as_encoded_bytes().len())
            .and_then(|bytes| {
                bytes.checked_add(self.executable.as_os_str().as_encoded_bytes().len())
            })
            .and_then(|bytes| {
                self.arguments
                    .len()
                    .checked_mul(std::mem::size_of::<OsString>())
                    .and_then(|arguments| bytes.checked_add(arguments))
            });
        for argument in &self.arguments {
            let length = match argument.as_str() {
                "{left}" => paths[0].as_os_str().as_encoded_bytes().len(),
                "{right}" => paths[1].as_os_str().as_encoded_bytes().len(),
                _ => argument.len(),
            };
            bytes = bytes.and_then(|bytes| bytes.checked_add(length));
        }
        if bytes.is_none_or(|bytes| bytes > 8 * 1024 * 1024) {
            return Err("Native process data exceeds its 8 MiB workload budget.");
        }
        let arguments = self.arguments_for(paths)?;
        let process = UserCommandProcess {
            working_directory,
            invocation: UserCommandProcessInvocation::Program {
                executable: self.executable.clone(),
                arguments,
            },
        };
        process.validate()?;
        Ok(ExternalCompareProcess(process))
    }
}

fn native_absolute(path: &Path) -> bool {
    path.is_absolute() && !path.as_os_str().as_encoded_bytes().contains(&0)
}

fn diagnostic(field: FileComparatorField, message: &'static str) -> FileComparatorDiagnostic {
    FileComparatorDiagnostic {
        rule: None,
        field,
        message: message.into(),
    }
}

/// Immutable Program-only process data; construction validates literal native argv.
pub struct ExternalCompareProcess(UserCommandProcess);

impl ExternalCompareProcess {
    pub fn as_process(&self) -> &UserCommandProcess {
        &self.0
    }
}

impl fmt::Debug for ExternalCompareProcess {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("ExternalCompareProcess")
            .field("invocation", &self.0.invocation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub enum ExternalCompareOutcome {
    NotStarted(FsError),
    Started {
        pid: u32,
        tracking_warning: Option<String>,
    },
    Unknown {
        message: String,
    },
}

#[derive(Clone, Debug)]
pub struct ComparatorImageCapability {
    pub provider_id: String,
    pub extensions: Vec<String>,
}

/// Metadata-only coverage, not a promise that any particular image will decode.
#[derive(Clone, Debug)]
pub struct ComparatorCapabilities {
    pub revision: u64,
    pub disabled_plugins: Arc<[String]>,
    pub images: Result<Arc<[ComparatorImageCapability]>, String>,
}
