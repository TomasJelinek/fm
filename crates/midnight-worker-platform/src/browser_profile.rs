//! Private, session-only Chromium state. No personal browser profile is ever reused.
use crate::store::PrivateStore;
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};
static NEXT: AtomicU64 = AtomicU64::new(1);

pub(super) struct Profile {
    path: PathBuf,
    session: Option<PrivateStore>,
    _parent: PrivateStore,
}
impl Profile {
    pub fn new(parent: &Path) -> io::Result<Self> {
        let root = PrivateStore::open(parent)?;
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path = parent.join(format!(
            "session-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let session = PrivateStore::open(&path)?;
        Ok(Self {
            path,
            session: Some(session),
            _parent: root,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        // Release the final Windows directory pin while retaining its private parent/ancestors.
        // remove_dir_all does not follow child symlinks. Only this fresh owned session is removed.
        self.session.take();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
