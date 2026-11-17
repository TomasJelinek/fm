//! Complete original transport ownership, armed before admission becomes visible.
use super::*;

pub(super) struct Submission {
    command: Option<Command>,
    pub(super) id: OperationId,
}

impl Submission {
    pub(super) fn new(command: Command) -> Self {
        let id = command_id(&command);
        Self {
            command: Some(command),
            id,
        }
    }
    pub(super) fn take_command(&mut self) -> Command {
        self.command
            .take()
            .expect("Copy submission owns its original command")
    }
}

impl crate::copy_nested_retirement::CheckedRetirement for Submission {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Drop for Submission {
    fn drop(&mut self) {
        if let Some(command) = self.command.take() {
            // The constructors move captures into explicit consuming owners and
            // invoke no accessors. Scope runs this Drop under the Copy context.
            match CopyInput::from_command(command) {
                Ok(input) => drop(input),
                Err(command) => {
                    std::mem::forget(command);
                    if let Some(context) = crate::copy_nested_retirement::current() {
                        context.quarantine();
                    }
                }
            }
        }
    }
}

fn command_id(command: &Command) -> OperationId {
    fn metadata_id(request: &MetadataTransferRequest) -> OperationId {
        match &request.input {
            MetadataTransferInput::Paths(request) => request.id,
            MetadataTransferInput::Selected(request) => request.id,
            MetadataTransferInput::Clipboard(request) => request.id,
        }
    }
    match command {
        Command::EnqueueCopy(request) | Command::EnqueueMove(request) => request.id,
        Command::EnqueueSelectedCopy(request) | Command::EnqueueSelectedMove(request) => request.id,
        Command::EnqueueClipboardMove(request) => request.id,
        Command::EnqueueTransferWithMetadata(request) => metadata_id(request),
        Command::EnqueueCopyWithOptions(request) => match &request.input {
            CopyOptionsInput::Native(request) => metadata_id(request),
            CopyOptionsInput::Captured(request) => request.id,
        },
        Command::EnqueueArchiveCopy(request) => request.id,
        Command::EnqueueCapturedTransfer(request) => request.id,
        _ => unreachable!("only Copy-family commands enter guarded admission"),
    }
}

#[cfg(test)]
#[path = "copy_submission_tests.rs"]
mod tests;
