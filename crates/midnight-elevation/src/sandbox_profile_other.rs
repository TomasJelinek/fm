//! Only Linux refuses Chromium's sandbox in a way that a profile installed with
//! authorization allows, so no other platform offers one.
use crate::SandboxProfileError;
use midnight_core::Cancellation;
use std::{convert::Infallible, path::Path};

/// Never offered on this platform: no value exists.
pub struct SandboxProfile {
    never: Infallible,
}

impl SandboxProfile {
    /// Always `None`: nothing to install on this platform.
    pub fn for_browser(_executable: &Path) -> Option<Self> {
        None
    }

    pub fn attachment(&self) -> &str {
        match self.never {}
    }

    pub fn changeable(&self) -> bool {
        match self.never {}
    }

    pub fn install(&self, _cancel: &Cancellation) -> Result<(), SandboxProfileError> {
        match self.never {}
    }
}
