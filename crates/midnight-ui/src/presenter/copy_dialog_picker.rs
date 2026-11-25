//! A chosen folder updates the current draft's field. Confirmation still checks its destination.
use super::*;

pub(super) fn resolve_destination(
    proposed: &Path,
    picked: Option<&(DesktopRequestId, PathBuf)>,
    edited: &str,
) -> Result<PathBuf, &'static str> {
    // The latest native selection wins even when distinct native names display identically.
    let displayed = picked.map_or(proposed, |(_, path)| path.as_path());
    if let Some(original) = [displayed, proposed]
        .into_iter()
        .find(|original| original.to_string_lossy() == edited)
    {
        validate_picked_path(original).map_err(|_| DESTINATION_FOLDER_REQUIRED)?;
        return Ok(original.to_owned());
    }
    if crate::native_path_edit::ambiguous_edit(displayed, edited)
        || crate::native_path_edit::ambiguous_edit(proposed, edited)
    {
        return Err(concat!(
            "The displayed destination folder contains characters this editor cannot represent. ",
            "Enter another complete path without replacement characters."
        ));
    }
    let path = PathBuf::from(edited);
    validate_picked_path(&path).map_err(|_| DESTINATION_FOLDER_REQUIRED)?;
    Ok(path)
}

impl Presenter {
    pub(crate) fn browse_copy_destination(
        &mut self,
        token: &str,
        edited: &str,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(draft) = self.copies.draft.as_mut().filter(|draft| {
            draft.token == token && !draft.retire_requested && !self.operations.closing
        }) else {
            return;
        };
        if self.copies.destination_picker.is_some() {
            return;
        }
        let picked = self.copies.picked_destination.as_ref();
        let directory =
            resolve_destination(&draft.destination, picked, edited).unwrap_or_else(|_| {
                picked.map_or_else(|| draft.destination.clone(), |(_, p)| p.clone())
            });
        let request = self.desktop.request();
        if send(Command::PickPath {
            request,
            input: PathPickerRequest {
                kind: PathPickerKind::Directory,
                title: "Choose destination folder".into(),
                initial_directory: Some(directory),
                initial_name: None,
                owner,
            },
        }) {
            // An earlier OK may still be checking a different typed path. Its reply
            // must never start a transfer after the user has reopened the destination.
            if let Some(check) = self.copies.destination_check.take() {
                check.cancel.cancel();
            }
            self.copies.destination_picker = Some(request);
        } else {
            draft.error = "The folder chooser is busy. Try Browse again.".into();
        }
    }

    pub(crate) fn copy_destination_picked(
        &mut self,
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    ) {
        if self.copies.destination_picker != Some(request) {
            return;
        }
        self.copies.destination_picker = None;
        let Some(draft) = self.copies.draft.as_mut().filter(|d| !d.retire_requested) else {
            return;
        };
        if self.operations.closing {
            return;
        }
        match result {
            Ok(Some(path)) => match validate_picked_path(&path) {
                Ok(()) => {
                    self.copies.picked_destination = Some((request, path));
                    draft.error.clear();
                }
                Err(error) => draft.error = crate::error_presentation::describe(&error),
            },
            Ok(None) => {}
            Err(error) => draft.error = crate::error_presentation::describe(&error),
        }
    }

    pub(crate) fn cancel_copy_destination_picker(
        &mut self,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if let Some(request) = self.copies.destination_picker {
            if !send(Command::CancelPathPicker { request }) {
                return false;
            }
            self.copies.destination_picker = None;
        }
        true
    }
}
