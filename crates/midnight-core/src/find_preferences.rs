//! Saved Find placement and exact, bounded filename-mask history.
use crate::WindowGeometry;
use std::collections::HashSet;

pub const MAX_FIND_MASK_HISTORY: usize = 30;
pub const MAX_FIND_MASK_BYTES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindPreferences {
    pub window_geometry: Option<WindowGeometry>,
    /// Newest accepted search first; masks retain their exact Unicode spelling.
    pub mask_history: Vec<String>,
}

impl FindPreferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(geometry) = &self.window_geometry {
            geometry.validate()?;
        }
        if self.mask_history.len() > MAX_FIND_MASK_HISTORY {
            return Err("At most 30 Find masks can be remembered.");
        }
        let mut seen = HashSet::new();
        for mask in &self.mask_history {
            validate_mask(mask)?;
            if !seen.insert(mask) {
                return Err("Saved Find masks must be unique.");
            }
        }
        Ok(())
    }

    /// Remember an accepted search. Invalid input leaves the history unchanged.
    pub fn remember_mask(&mut self, mask: &str) -> Result<(), &'static str> {
        self.validate()?;
        let mask = if mask.is_empty() { "*.*" } else { mask };
        validate_mask(mask)?;
        self.mask_history.retain(|previous| previous != mask);
        self.mask_history.insert(0, mask.to_owned());
        self.mask_history.truncate(MAX_FIND_MASK_HISTORY);
        Ok(())
    }
}

fn validate_mask(mask: &str) -> Result<(), &'static str> {
    if mask.is_empty() || mask.len() > MAX_FIND_MASK_BYTES || mask.contains('\0') {
        return Err("Saved Find masks must contain 1 through 4096 UTF-8 bytes without NUL.");
    }
    Ok(())
}

#[cfg(test)]
#[path = "find_preferences_tests.rs"]
mod tests;
