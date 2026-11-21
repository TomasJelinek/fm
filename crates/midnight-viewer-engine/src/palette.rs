//! Window-scoped latest palette; no typography, source, renderer or I/O lock.
use midnight_core::ViewerAppearance;
use std::sync::{Arc, Mutex, mpsc::TrySendError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Colors {
    foreground: [u8; 4],
    background: [u8; 4],
    selection: [u8; 4],
    selection_text: [u8; 4],
    accent: [u8; 4],
}

impl From<&ViewerAppearance> for Colors {
    fn from(value: &ViewerAppearance) -> Self {
        Self {
            foreground: value.foreground,
            background: value.background,
            selection: value.selection,
            selection_text: value.selection_text,
            accent: value.accent,
        }
    }
}

impl Colors {
    /// Whether text, page or accent colors differ from `appearance`. Pages are laid
    /// out with those; the selection pair is only painted, so changing it repaints.
    pub fn recolors_text(self, appearance: &ViewerAppearance) -> bool {
        self.foreground != appearance.foreground
            || self.background != appearance.background
            || self.accent != appearance.accent
    }

    pub fn apply(self, appearance: &mut ViewerAppearance) {
        appearance.foreground = self.foreground;
        appearance.background = self.background;
        appearance.selection = self.selection;
        appearance.selection_text = self.selection_text;
        appearance.accent = self.accent;
    }
}

pub(crate) struct State {
    value: Mutex<Accepted>,
}

struct Accepted {
    revision: u64,
    colors: Colors,
    closed: bool,
    stopped: bool,
}

impl State {
    pub fn new(appearance: &ViewerAppearance) -> Self {
        Self {
            value: Mutex::new(Accepted {
                revision: 1,
                colors: Colors::from(appearance),
                closed: false,
                stopped: false,
            }),
        }
    }

    pub fn snapshot(&self) -> (u64, Colors) {
        let value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        (value.revision, value.colors)
    }

    pub fn restore(
        &self,
        appearance: &ViewerAppearance,
        wake: impl FnOnce() -> Result<(), TrySendError<crate::worker::Message>>,
    ) -> Result<u64, String> {
        let colors = Colors::from(appearance);
        let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if value.closed || value.stopped {
            return Err("Viewer is closed.".into());
        }
        let revision = if colors == value.colors {
            value.revision
        } else {
            value
                .revision
                .checked_add(1)
                .ok_or("Viewer palette revision exhausted.")?
        };
        // A wake may race a worker snapshot, which takes this same short lock.
        // Full retains the latest slot; Disconnected rejects without changing it.
        if matches!(wake(), Err(TrySendError::Disconnected(_))) {
            return Err("Viewer worker is unavailable.".into());
        }
        value.revision = revision;
        value.colors = colors;
        Ok(revision)
    }

    pub fn close(&self) {
        self.value
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
    }
}

/// Captured before resident enqueue, so refusal/skipping also ends acceptance.
pub(crate) struct Lifetime {
    state: Arc<State>,
}

impl Lifetime {
    pub fn new(state: Arc<State>) -> Self {
        Self { state }
    }
    pub fn state(&self) -> Arc<State> {
        self.state.clone()
    }
}

impl Drop for Lifetime {
    fn drop(&mut self) {
        self.state
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped = true;
    }
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
