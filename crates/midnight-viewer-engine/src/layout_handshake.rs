//! One bounded, generation-scoped layout decision before content startup.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub token: u64,
    pub mode: u16,
    pub content_generation: u64,
}

#[derive(Default)]
pub(crate) struct Handshake {
    enabled: bool,
    next_token: u64,
    pending: Option<Selection>,
    closed: bool,
}

impl Handshake {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    pub fn select(
        &mut self,
        mode: u16,
        content_generation: u64,
    ) -> Result<Option<Selection>, String> {
        self.invalidate();
        if self.closed {
            return Err("Viewer closed before displaying content.".into());
        }
        if !self.enabled {
            return Ok(None);
        }
        let token = self
            .next_token
            .checked_add(1)
            .ok_or("Viewer layout token exhausted.")?;
        self.next_token = token;
        let selection = Selection {
            token,
            mode,
            content_generation,
        };
        self.pending = Some(selection);
        Ok(Some(selection))
    }

    pub fn accept(&mut self, token: u64) -> Option<Selection> {
        if self.pending.is_some_and(|selected| selected.token == token) {
            self.pending.take()
        } else {
            None
        }
    }

    pub fn invalidate(&mut self) {
        self.pending = None;
    }

    pub fn close(&mut self) {
        self.invalidate();
        self.closed = true;
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(test)]
    pub fn selection(&self) -> Option<Selection> {
        self.pending
    }
}

#[cfg(test)]
#[path = "layout_handshake_tests.rs"]
mod tests;
