use midnight_core::{ArchiveCompareMember, EntryKind};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) fn check(member: &ArchiveCompareMember) -> Result<(), String> {
    let node = &member.node;
    let folder = &member.folder;
    if node.kind != EntryKind::File
        || !node.readable
        || node.size.is_none()
        || node.id.mount != folder.mount.id()
        || node.id.entry == 0
        || node.parent != folder.directory
        || folder.directory == 0
        || folder.source_version.is_empty()
    {
        return Err("The archive member is unavailable in the captured folder.".into());
    }
    Ok(())
}

pub(crate) fn capture(
    member: &ArchiveCompareMember,
    cancel: &Arc<AtomicBool>,
) -> Result<CapturedFile, String> {
    check(member)?;
    let content = MemberContent {
        member: member.clone(),
        length: member
            .node
            .size
            .ok_or("The archive member has no known length.")?,
        cancel: cancel.clone(),
    };
    CapturedFile::from_source(Arc::new(content)).map_err(|error| error.to_string())
}

#[derive(Debug)]
struct MemberContent {
    member: ArchiveCompareMember,
    length: u64,
    cancel: Arc<AtomicBool>,
}

impl ContentSource for MemberContent {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        self.member.node.display_name.clone()
    }
    fn version(&self) -> Vec<u8> {
        let mut version = self.member.folder.source_version.clone();
        version.extend(self.member.node.id.mount.to_le_bytes());
        version.extend(self.member.node.id.entry.to_le_bytes());
        version
    }
    fn validate(&self) -> Result<(), Error> {
        crate::check_cancel(&self.cancel).map_err(Error)?;
        self.member
            .folder
            .mount
            .validate()
            .map_err(|error| Error(error.to_string()))
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.member
            .folder
            .mount
            .read_cooperative(self.member.node.id.entry, offset, length, &|| {
                self.cancel.load(Ordering::Acquire)
            })
            .map_err(|error| Error(error.to_string()))
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.member.folder.mount.cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::into_inner(self) {
            Some(source) => source
                .member
                .folder
                .mount
                .release_on_worker()
                .map_err(|error| Error(error.to_string())),
            None => Ok(()),
        }
    }
}
