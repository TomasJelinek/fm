//! Pause requests retain exact job identity; only the backend acknowledges a safe pause.
use super::*;

impl Presenter {
    pub(crate) fn set_job_paused(
        &mut self,
        token: &str,
        paused: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing {
            return;
        }
        if let Some(id) = token
            .strip_prefix("copy-job:")
            .and_then(|value| value.parse().ok())
            .map(OperationId)
        {
            if let Some(progress) = self.copies.progress.iter_mut().find(|progress| {
                progress.id == id
                    && !progress.state.is_terminal()
                    && progress.state != CopyOperationState::CancelRequested
            }) && send(Command::SetCopyPaused { id, paused })
            {
                Arc::make_mut(progress).pause_requested = paused;
            }
        } else if let Ok(id) = token.parse::<u64>().map(OperationId)
            && !self.operations.unconfirmed.contains(&id)
            && let Some(progress) = self.operations.progress.iter_mut().find(|progress| {
                progress.id == id
                    && matches!(progress.kind, MutationKind::Delete(_))
                    && !progress.state.is_terminal()
                    && progress.state != OperationState::CancelRequested
            })
            && send(Command::SetOperationPaused { id, paused })
        {
            Arc::make_mut(progress).pause_requested = paused;
        }
    }
}
