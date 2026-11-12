//! Whole-channel ownership includes sends admitted before receiver close.
use midnight_core::Command;
use std::ops::{Deref, DerefMut};
use tokio::sync::mpsc;

/// Must remain inside WorkerOwned: draining can wait for an outstanding send permit.
pub(crate) struct CommandInbox(pub(crate) mpsc::Receiver<Command>);
impl Deref for CommandInbox {
    type Target = mpsc::Receiver<Command>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for CommandInbox {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for CommandInbox {
    fn drop(&mut self) {
        self.0.close();
        // Tokio's plain Receiver::drop cannot retain future sends from existing
        // permits. The reserved plain worker drains those too before releasing its
        // ownership slot; no caller/executor waits for the producer.
        while let Some(command) = self.0.blocking_recv() {
            drop(command);
        }
    }
}
