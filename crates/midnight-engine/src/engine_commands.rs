//! Startup-owned commands with Copy admission before successful transport acceptance.
use crate::{
    copy_operations::{Admission, Admitted},
    retirement::{RetirementLease, WorkerOwned},
};
use midnight_core::Command;
use tokio::sync::mpsc;

#[cfg_attr(
    windows,
    expect(
        clippy::large_enum_variant,
        reason = "Store plain commands inline in the bounded FIFO to avoid a box allocation per send."
    )
)]
pub(crate) enum Message {
    Plain(PlainCommand),
    GuardedCopy(Admitted),
}
pub(crate) struct PlainCommand(Command);
impl PlainCommand {
    pub(crate) fn into_command(self) -> Command {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandChannelError {
    AlreadyConfigured,
    InvalidCapacity,
    Closed,
}
impl std::fmt::Display for CommandChannelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyConfigured => "The Engine command channel was already created.",
            Self::InvalidCapacity => "The Engine command channel capacity must be positive.",
            Self::Closed => "The Engine command channel is closed.",
        })
    }
}
impl std::error::Error for CommandChannelError {}

/// Full or closed refusal retaining the complete command inline.
/// The caller remains responsible for backend reclamation of captured owners.
pub type CommandSendError = mpsc::error::TrySendError<Command>;

/// The production command FIFO. Copy submissions use the existing 32-job bound.
///
/// Failed sends return the complete caller-owned command. Callers must retain or
/// reclaim native captures on their existing backend reclamation path.
#[derive(Clone)]
pub struct CommandSender {
    sender: mpsc::Sender<Message>,
    admission: Admission,
}

/// Reserved FIFO capacity; sending a Copy also requires its actual admission.
pub struct OwnedCommandPermit {
    permit: mpsc::OwnedPermit<Message>,
    admission: Admission,
}

/// Borrowed channel credit with the same admitted send path as an owned credit.
pub struct CommandPermit<'a> {
    owned: OwnedCommandPermit,
    _borrow: std::marker::PhantomData<&'a CommandSender>,
}

/// Opaque receiver already protected by its originating Engine's cleanup slot.
pub struct CommandReceiver {
    pub(crate) inbox: WorkerOwned<CommandInbox>,
    pub(crate) admission: Admission,
}

impl CommandSender {
    pub(crate) fn channel(
        capacity: usize,
        admission: Admission,
        lease: RetirementLease,
    ) -> (Self, CommandReceiver) {
        let (sender, receiver) = mpsc::channel(capacity);
        let inbox = lease.own(CommandInbox(receiver));
        (
            Self {
                sender,
                admission: admission.clone(),
            },
            CommandReceiver { inbox, admission },
        )
    }

    /// Accept a command without waiting. Full/closed returns the intact command.
    /// Successful Copy acceptance already owns its payload and actual admission.
    #[expect(
        clippy::result_large_err,
        reason = "Keep refused commands intact for caller reclamation without an additional box."
    )]
    pub fn try_send(&self, command: Command) -> Result<(), CommandSendError> {
        let permit = match self.sender.clone().try_reserve_owned() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                return Err(mpsc::error::TrySendError::Full(command));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(mpsc::error::TrySendError::Closed(command));
            }
        };
        OwnedCommandPermit {
            permit,
            admission: self.admission.clone(),
        }
        .try_send(command)
    }

    /// Wait for FIFO room. Copy admission itself is nonblocking: a full Copy
    /// queue returns Full(command), preserving ownership for the caller.
    #[expect(
        clippy::result_large_err,
        reason = "Keep refused commands intact for caller reclamation without an additional box."
    )]
    pub async fn send(&self, command: Command) -> Result<(), CommandSendError> {
        let permit = match self.reserve().await {
            Ok(permit) => permit,
            Err(_) => return Err(mpsc::error::TrySendError::Closed(command)),
        };
        permit.try_send(command)
    }

    /// Reserve FIFO space only. The returned permit cannot bypass Copy admission.
    pub async fn reserve(&self) -> Result<CommandPermit<'_>, CommandChannelError> {
        let permit = self
            .sender
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| CommandChannelError::Closed)?;
        Ok(CommandPermit {
            owned: OwnedCommandPermit {
                permit,
                admission: self.admission.clone(),
            },
            _borrow: std::marker::PhantomData,
        })
    }

    /// Owned form of reserve, with the same Copy admission check at send time.
    pub async fn reserve_owned(self) -> Result<OwnedCommandPermit, CommandChannelError> {
        let permit = self
            .sender
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| CommandChannelError::Closed)?;
        Ok(OwnedCommandPermit {
            permit,
            admission: self.admission.clone(),
        })
    }

    /// Whether the Engine command receiver has closed.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Wait until the Engine command receiver closes.
    pub async fn closed(&self) {
        self.sender.closed().await;
    }
}

impl CommandPermit<'_> {
    /// Send through reserved FIFO space, preserving an intact refused command.
    #[expect(
        clippy::result_large_err,
        reason = "Keep refused commands intact for caller reclamation without an additional box."
    )]
    pub fn try_send(self, command: Command) -> Result<(), CommandSendError> {
        self.owned.try_send(command)
    }
}
impl OwnedCommandPermit {
    /// Send through reserved FIFO space, returning an intact refused Copy command.
    #[expect(
        clippy::result_large_err,
        reason = "Keep refused commands intact for caller reclamation without an additional box."
    )]
    pub fn try_send(self, command: Command) -> Result<(), CommandSendError> {
        if is_copy(&command) {
            self.admission.send_reserved(command, self.permit)
        } else {
            self.permit.send(Message::Plain(PlainCommand(command)));
            Ok(())
        }
    }
}

fn is_copy(command: &Command) -> bool {
    matches!(
        command,
        Command::EnqueueCopy(_)
            | Command::EnqueueSelectedCopy(_)
            | Command::EnqueueMove(_)
            | Command::EnqueueSelectedMove(_)
            | Command::EnqueueClipboardMove(_)
            | Command::EnqueueTransferWithMetadata(_)
            | Command::EnqueueCopyWithOptions(_)
            | Command::EnqueueArchiveCopy(_)
            | Command::EnqueueCapturedTransfer(_)
    )
}

/// Must remain WorkerOwned, including before the first run poll. Draining may
/// wait for producers that already hold a FIFO permit when the receiver closes.
pub(crate) struct CommandInbox(mpsc::Receiver<Message>);

impl CommandInbox {
    pub(crate) fn close(&mut self) {
        self.0.close();
    }
    pub(crate) async fn recv(&mut self) -> Option<Message> {
        self.0.recv().await
    }
}

impl Drop for CommandInbox {
    fn drop(&mut self) {
        self.0.close();
        while let Some(command) = self.0.blocking_recv() {
            drop(command);
        }
    }
}

#[cfg(test)]
#[path = "engine_command_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "copy_invalid_id_tests.rs"]
mod invalid_id_tests;
