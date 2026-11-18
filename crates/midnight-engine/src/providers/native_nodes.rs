use super::ProviderError;
use midnight_core::{EntryKind, NodeToken};
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    path::PathBuf,
    sync::Arc,
};

const MAX_NODES: usize = 100_000;
pub(super) const MAX_PATH_STORAGE: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct NativeNode {
    pub path: Arc<PathBuf>,
    pub kind: EntryKind,
}

#[derive(Default)]
pub(super) struct NativeNodes {
    by_path: HashMap<Arc<PathBuf>, NodeToken>,
    by_token: HashMap<NodeToken, NativeNode>,
    path_bytes: usize,
    next: u64,
}

impl NativeNodes {
    pub fn len(&self) -> usize {
        self.by_token.len()
    }

    #[cfg(test)]
    pub(super) fn path_bytes(&self) -> usize {
        self.path_bytes
    }

    #[cfg(test)]
    pub(super) fn capacities(&self) -> (usize, usize) {
        (self.by_token.capacity(), self.by_path.capacity())
    }

    pub fn get(&self, token: &NodeToken) -> Result<NativeNode, ProviderError> {
        self.by_token
            .get(token)
            .cloned()
            .ok_or(ProviderError::StaleReference)
    }

    pub fn bind(&mut self, path: PathBuf, kind: EntryKind) -> Result<NodeToken, ProviderError> {
        if let Some(token) = self.by_path.get(&path)
            && self
                .by_token
                .get(token)
                .is_some_and(|entry| entry.kind == kind)
        {
            return Ok(token.clone());
        }
        let bytes = self
            .path_bytes
            .saturating_add(path.as_os_str().as_encoded_bytes().len());
        if self.by_token.len() >= MAX_NODES || bytes > MAX_PATH_STORAGE {
            return Err(ProviderError::ResourceLimited);
        }
        let next = self
            .next
            .checked_add(1)
            .ok_or(ProviderError::ResourceLimited)?;
        let token = NodeToken::new(&next.to_le_bytes())
            .map_err(|error| ProviderError::Failed(error.to_string()))?;
        self.next = next;
        self.path_bytes = bytes;
        let path = Arc::new(path);
        self.by_path.insert(path.clone(), token.clone());
        self.by_token
            .insert(token.clone(), NativeNode { path, kind });
        Ok(token)
    }

    /// Forget every token outside `keep`. Kept tokens keep their path and kind,
    /// and each kept path is again reachable by `bind` through its newest kept
    /// token, as after that binding. `next` never moves back, so a forgotten
    /// token value is never minted again: a forgotten path bound later receives
    /// a new token.
    pub fn retain(&mut self, keep: &HashSet<NodeToken>) {
        self.by_token.retain(|token, _| keep.contains(token));
        super::super::release_spare_capacity(&mut self.by_token);
        let mut by_path = HashMap::with_capacity(self.by_token.len());
        let mut path_bytes = 0_usize;
        for (token, node) in &self.by_token {
            path_bytes = path_bytes.saturating_add(node.path.as_os_str().as_encoded_bytes().len());
            match by_path.entry(node.path.clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(token.clone());
                }
                // A kind change bound the same path again; the newest binding wins.
                Entry::Occupied(mut slot) => {
                    if serial(token) > serial(slot.get()) {
                        slot.insert(token.clone());
                    }
                }
            }
        }
        self.by_path = by_path;
        self.path_bytes = path_bytes;
    }
}

/// The counter value `bind` minted into a token.
fn serial(token: &NodeToken) -> u64 {
    token.as_bytes().try_into().map_or(0, u64::from_le_bytes)
}
