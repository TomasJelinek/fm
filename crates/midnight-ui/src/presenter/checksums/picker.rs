//! Native path choices are bound to one checksum session and its current generation.
use super::*;

pub(crate) struct Pending {
    request: DesktopRequestId,
    generation: ChecksumGeneration,
    verify: bool,
}

impl Presenter {
    pub(crate) fn request_checksum_picker(
        &mut self,
        id: ChecksumJobId,
        verify: bool,
        algorithm: ChecksumAlgorithm,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing {
            return;
        }
        let request = self.desktop.request();
        let Some(s) = self.checksums.session_mut(id) else {
            return;
        };
        if s.picker.is_some() || s.save.is_some() || s.running {
            return;
        }
        let path = if verify {
            &s.manifest_suggested
        } else {
            &s.suggested
        };
        if send(Command::PickPath {
            request,
            input: PathPickerRequest {
                kind: if verify {
                    PathPickerKind::ChecksumOpen
                } else {
                    PathPickerKind::ChecksumSave(algorithm)
                },
                title: if verify {
                    "Verify checksum file"
                } else {
                    "Save checksum file"
                }
                .into(),
                initial_directory: path.parent().map(Path::to_owned),
                owner,
            },
        }) {
            s.picker = Some(Pending {
                request,
                generation: s.generation,
                verify,
            });
            s.error.clear();
        } else {
            s.error = "File chooser queue is busy. Try Browse again.".into();
        }
    }

    pub(crate) fn owns_checksum_picker(&self, request: DesktopRequestId) -> bool {
        self.checksums
            .sessions
            .values()
            .any(|s| s.picker.as_ref().is_some_and(|p| p.request == request))
    }

    pub(crate) fn checksum_path_picked(
        &mut self,
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    ) {
        let id = self.checksums.sessions.iter().find_map(|(id, s)| {
            s.picker
                .as_ref()
                .filter(|p| p.request == request)
                .map(|_| *id)
        });
        let Some(s) = id.and_then(|id| self.checksums.session_mut(id)) else {
            self.retired.push(Box::new(result));
            return;
        };
        let Some(pending) = s.picker.take() else {
            return;
        };
        if pending.generation != s.generation || self.operations.closing {
            self.retired.push(Box::new(result));
            return;
        }
        match result {
            Ok(Some(path)) => {
                if let Err(error) = validate_picked_path(&path) {
                    s.error = crate::error_presentation::describe(&error);
                    return;
                }
                if pending.verify {
                    s.manifest_suggested = path;
                    s.manifest_revision = s.manifest_revision.wrapping_add(1);
                } else {
                    s.suggested = path;
                    s.destination_revision = s.destination_revision.wrapping_add(1);
                }
                s.error.clear();
            }
            Ok(None) => {}
            Err(error) => s.error = crate::error_presentation::describe(&error),
        }
    }

    pub(super) fn cancel_checksum_picker(
        &mut self,
        id: ChecksumJobId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(s) = self.checksums.session_mut(id) else {
            return true;
        };
        let Some(pending) = &s.picker else {
            return true;
        };
        if !send(Command::CancelPathPicker {
            request: pending.request,
        }) {
            s.error = "Request queue is busy. Try closing again.".into();
            return false;
        }
        s.picker = None;
        true
    }
}
