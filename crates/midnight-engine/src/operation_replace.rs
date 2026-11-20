//! A confirmed Shift+F4 over an existing file: the file is replaced with an empty one
//! through verified staging and the platform's recoverable replacement, in the folder
//! pinned when the New empty file dialog opened. The original stays recoverable, and
//! the job says where so the user can find and remove it.
use super::*;
use crate::verified_output::{publish, with_io};
use std::path::PathBuf;

/// A completed replacement as its job reports it.
pub(super) struct Replaced {
    pub(super) path: PathBuf,
    /// Where the original is kept, for the job's message. Empty when nothing was
    /// displaced or when `issue` already says it.
    pub(super) note: String,
    /// The empty file is in place, but a later step failed. Its message also says
    /// where the original is kept.
    pub(super) issue: Option<FsError>,
}

pub(super) fn replace_with_empty_file(
    service: &dyn TransferService,
    base: &dyn PreparedDirectory,
    folder: &Path,
    name: &std::ffi::OsStr,
    cancel: &Cancellation,
) -> Result<Replaced, FsError> {
    let path = folder.join(name);
    let (directory, target) = with_io(&path, cancel, || {
        let directory = service.prepare_transfer_destination(folder, cancel)?;
        // The transfer capability is opened by path; it must reach the folder the job
        // pinned, or a moved or replaced folder would retarget the confirmed change.
        if directory.identity() != base.transfer_identity(cancel)? {
            return Err(error(
                folder,
                "The folder was moved or replaced after the New empty file dialog opened. \
                 Nothing was replaced.",
            ));
        }
        let target = directory.prepare_target(name, cancel)?;
        Ok((directory, target))
    })?;
    if target
        .as_ref()
        .is_some_and(|target| target.entry().item.kind != EntryKind::File)
    {
        return Err(error(
            &path,
            "Only a file can be replaced with an empty file.",
        ));
    }
    let published = publish(
        directory.as_ref(),
        &path,
        target.as_ref(),
        Vec::new(),
        cancel,
    )?;
    let note = recovery_note(&published.recovery);
    Ok(match published.warning {
        Some(mut issue) => {
            if !note.is_empty() {
                issue.message.push('\n');
                issue.message.push_str(&note);
            }
            Replaced {
                path: published.path,
                note: String::new(),
                issue: Some(issue),
            }
        }
        None => Replaced {
            note: if note.is_empty() {
                note
            } else {
                format!(
                    "Replaced {} with an empty file.\n{note}",
                    published.path.display()
                )
            },
            path: published.path,
            issue: None,
        },
    })
}

/// Where a replacement kept the original, and that it is the user's to remove.
fn recovery_note(recovery: &[PathBuf]) -> String {
    let (kept, remove) = match recovery {
        [] => return String::new(),
        [_] => (
            "The original file is kept for recovery at",
            "Delete it when you no longer need it.",
        ),
        _ => (
            "The original files are kept for recovery at",
            "Delete them when you no longer need them.",
        ),
    };
    let mut note = kept.to_owned();
    for path in recovery {
        note.push('\n');
        note.push_str(&path.display().to_string());
    }
    note.push('\n');
    note.push_str(remove);
    note
}
