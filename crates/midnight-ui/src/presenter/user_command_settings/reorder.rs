//! Immutable configuration capture and bounded ordering, without persistence or UI work.
use midnight_core::{MAX_USER_COMMANDS, UserCommand, UserCommandId};
use std::sync::{Arc, Weak};

pub(super) struct Drag {
    id: UserCommandId,
    source: Weak<Vec<UserCommand>>,
    revision: u64,
}
impl Drag {
    pub(super) fn new(
        commands: &Arc<Vec<UserCommand>>,
        revision: u64,
        id: &str,
    ) -> Result<Self, &'static str> {
        if commands.len() > MAX_USER_COMMANDS {
            return Err("Custom Commands exceed the 128-row ordering budget.");
        }
        let mut matching = commands.iter().filter(|command| command.id.as_str() == id);
        let command = matching.next().ok_or("This command no longer exists.")?;
        if matching.next().is_some() {
            return Err("This command identity is ambiguous.");
        }
        Ok(Self {
            id: command.id.clone(),
            source: Arc::downgrade(commands),
            revision,
        })
    }

    pub(super) fn order(
        &self,
        commands: &Arc<Vec<UserCommand>>,
        revision: u64,
        dirty: bool,
        boundary: usize,
    ) -> Result<Option<Vec<UserCommand>>, &'static str> {
        self.validate(commands, revision, dirty)?;
        if boundary > commands.len() {
            return Err("This command drop position is no longer available.");
        }
        let source = commands
            .iter()
            .position(|command| command.id == self.id)
            .ok_or("This command no longer exists.")?;
        let target = boundary - usize::from(boundary > source);
        if target == source {
            return Ok(None);
        }
        let mut ordered = commands.as_ref().clone();
        let command = ordered.remove(source);
        ordered.insert(target, command);
        Ok(Some(ordered))
    }

    pub(super) fn id(&self) -> &UserCommandId {
        &self.id
    }

    pub(super) fn validate(
        &self,
        commands: &Arc<Vec<UserCommand>>,
        revision: u64,
        dirty: bool,
    ) -> Result<(), &'static str> {
        if dirty {
            return Err("Apply or cancel the current draft before reordering commands.");
        }
        if self.revision != revision || self.source.as_ptr() != Arc::as_ptr(commands) {
            return Err("Custom Commands changed. Start the drag again.");
        }
        if commands.len() > MAX_USER_COMMANDS
            || !commands.iter().any(|command| command.id == self.id)
        {
            return Err("This command no longer exists.");
        }
        Ok(())
    }
}
