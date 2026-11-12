use super::ProviderError;
use midnight_core::{EntryKind, NodeToken};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

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
}
