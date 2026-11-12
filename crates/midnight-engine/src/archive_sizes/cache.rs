use super::{Observation, scan::MAX_NODES};
use crate::tree_roots::ReadyRoot;
use midnight_core::{
    ArchiveMount, ArchiveNode, EntryId, EntryKind, FolderSize, SelectedFolderSizes, VirtualEntryId,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

pub(super) struct Cache {
    pub mount: Weak<dyn ArchiveMount>,
    pub id: u64,
    pub values: HashMap<VirtualEntryId, FolderSize>,
}
#[derive(Default)]
pub(super) struct Store {
    caches: Vec<Cache>,
}
impl Store {
    pub fn get(&mut self, mount: &Arc<dyn ArchiveMount>, pinned: Option<u64>) -> &mut Cache {
        if let Some(index) = self.caches.iter().position(|cache| cache.id == mount.id()) {
            let cache = &mut self.caches[index];
            if !cache
                .mount
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, mount))
            {
                cache.values.clear();
                cache.mount = Arc::downgrade(mount);
            }
            return cache;
        }
        if self.caches.len() == 2 {
            let index = self
                .caches
                .iter()
                .position(|cache| Some(cache.id) != pinned)
                .unwrap_or(0);
            self.caches.remove(index);
        }
        self.caches.push(Cache {
            mount: Arc::downgrade(mount),
            id: mount.id(),
            values: HashMap::new(),
        });
        let index = self.caches.len() - 1;
        &mut self.caches[index]
    }
}

pub(super) fn targets(
    ready: &ReadyRoot,
    selected: Option<&HashSet<EntryId>>,
) -> Result<Vec<ArchiveNode>, &'static str> {
    let location = ready.archive().ok_or("Archive source is unavailable.")?;
    let mount = location
        .mount
        .as_ref()
        .ok_or("Archive source is unavailable.")?;
    let snapshot = ready.snapshot();
    if mount.id() == 0
        || location.directory == 0
        || snapshot.entries.len() > MAX_NODES
        || snapshot.index.by_id.len() != snapshot.entries.len()
    {
        return Err("Archive listing has invalid size-scan identities.");
    }
    if selected.is_some_and(|ids| ids.iter().any(|id| !snapshot.index.by_id.contains_key(id))) {
        return Err("Archive size request is outside its captured flat listing.");
    }
    let mut seen: HashSet<_> = location.trail.iter().map(|(id, _)| *id).collect();
    if location.trail.len() > 257
        || location.trail.first().map(|(id, _)| *id) != Some(1)
        || location.trail.last().map(|(id, _)| *id) != Some(location.directory)
        || seen.len() != location.trail.len()
    {
        return Err("Archive listing has invalid ancestor identities.");
    }
    let mut result = Vec::new();
    for (index, entry) in snapshot.entries.iter().enumerate() {
        let node = entry
            .virtual_node
            .as_ref()
            .ok_or("Native row in an archive listing.")?;
        if !entry.path.as_os_str().is_empty()
            || node.id.mount != mount.id()
            || node.id.entry <= 1
            || node.parent != location.directory
            || entry.id.0 != node.id.entry
            || entry.kind != node.kind
            || entry.metadata.size != node.size
            || !seen.insert(node.id.entry)
            || snapshot.index.by_id.get(&entry.id) != Some(&index)
        {
            return Err("Archive listing has invalid size-scan identities.");
        }
        if entry.kind == EntryKind::Directory && selected.is_none_or(|ids| ids.contains(&entry.id))
        {
            result.push(node.clone());
        }
    }
    Ok(result)
}

pub(super) fn summary(observation: &Observation, cache: &Cache) -> SelectedFolderSizes {
    let mut result = SelectedFolderSizes {
        complete_bytes: Some(0),
        partial_bytes: Some(0),
        ..SelectedFolderSizes::default()
    };
    let (mut complete, mut partial) = (0u128, 0u128);
    let Some(ready) = observation.ready.upgrade() else {
        return result;
    };
    for entry in ready.snapshot().entries.iter() {
        if !observation.selected.contains(&entry.id)
            || !matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
        {
            continue;
        }
        result.folders += 1;
        let Some(node) = &entry.virtual_node else {
            continue;
        };
        let Some(size) = cache
            .values
            .get(&node.id)
            .filter(|_| entry.kind == EntryKind::Directory)
        else {
            continue;
        };
        if size.finished && size.errors == 0 {
            result.complete += 1;
            complete += u128::from(size.bytes);
        } else {
            result.partial += 1;
            partial += u128::from(size.bytes);
        }
    }
    result.complete_bytes = u64::try_from(complete).ok();
    result.partial_bytes = u64::try_from(partial).ok();
    result
}
