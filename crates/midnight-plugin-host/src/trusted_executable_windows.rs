//! Backend-only executable identity under a continuous read-data/write-delete-deny lease.
//! Package/path trust is checked by the common owner before and after revalidation.
use crate::{Error, native};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    fs::File,
    io,
    mem::ManuallyDrop,
    os::windows::{
        fs::MetadataExt,
        io::{AsRawHandle, RawHandle},
    },
    path::Path,
    sync::Mutex,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetHandleInformation, HANDLE_FLAG_INHERIT},
    Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
};

pub(super) struct ExecutableLease {
    owner: FileOwner,
    initial: native::Snapshot,
    probe_unconfirmed: Mutex<bool>,
    #[cfg(test)]
    probe_close: fn(RawHandle) -> io::Result<()>,
    #[cfg(test)]
    probe_opens: AtomicUsize,
}

impl ExecutableLease {
    pub(super) fn capture(executable: &Path) -> Result<Self, Error> {
        let owner = FileOwner::open(executable)?;
        match checked_snapshot(owner.file()) {
            Ok(initial) => Ok(Self {
                owner,
                initial,
                probe_unconfirmed: Mutex::new(false),
                #[cfg(test)]
                probe_close: close_handle,
                #[cfg(test)]
                probe_opens: AtomicUsize::new(0),
            }),
            Err(error) => release_after(Err(error), owner.release_on_worker()),
        }
    }

    pub(super) fn ensure_confirmed(&self) -> Result<(), Error> {
        let unconfirmed = self
            .probe_unconfirmed
            .lock()
            .map_err(|_| unconfirmed_probe())?;
        if *unconfirmed {
            return Err(unconfirmed_probe());
        }
        Ok(())
    }

    pub(super) fn revalidate(&self, executable: &Path) -> Result<(), Error> {
        // Serialize all probes, including the sticky check before any native inspection.
        let mut unconfirmed = self
            .probe_unconfirmed
            .lock()
            .map_err(|_| unconfirmed_probe())?;
        if *unconfirmed {
            return Err(unconfirmed_probe());
        }
        self.check_file(self.owner.file())?;
        let named = FileOwner::open(executable)?;
        // Arm immediately after open. Any error/unwind, including fallback close,
        // leaves this set; only a confirmed explicit close may clear it.
        *unconfirmed = true;
        #[cfg(test)]
        let named = {
            self.probe_opens.fetch_add(1, Ordering::Relaxed);
            let mut named = named;
            named.close = self.probe_close;
            named
        };
        let result = self.check_file(named.file());
        let close = named.release_on_worker();
        if close.is_ok() {
            *unconfirmed = false;
        }
        release_after(result, close)
    }

    fn check_file(&self, file: &File) -> Result<(), Error> {
        let current = checked_snapshot(file)?;
        if !self.initial.matches_leased_executable(&current) {
            return Err(Error(format!(
                "Trusted executable identity or content metadata changed: {}",
                self.initial.changed_fields(&current),
            )));
        }
        Ok(())
    }

    pub(super) fn release_on_worker(self) -> Result<(), Error> {
        // Poison is also unconfirmed. Retire the held owner once regardless, but
        // never acknowledge cleanup while an earlier probe outcome is unknown.
        let result = if self.probe_unconfirmed.into_inner().unwrap_or(true) {
            Err(unconfirmed_probe())
        } else {
            Ok(())
        };
        release_after(result, self.owner.release_on_worker())
    }
}

fn unconfirmed_probe() -> Error {
    Error("Trusted executable named probe cleanup is unconfirmed".into())
}

fn checked_snapshot(file: &File) -> Result<native::Snapshot, Error> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error(
            "Trusted executable needs a regular non-reparse file".into(),
        ));
    }
    let mut flags = 0;
    // SAFETY: file owns a live handle and flags is writable for the required u32.
    if unsafe { GetHandleInformation(file.as_raw_handle(), &mut flags) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    if flags & HANDLE_FLAG_INHERIT != 0 {
        return Err(Error(
            "Trusted executable lease must not be inherited".into(),
        ));
    }
    Ok(native::snapshot(file)?)
}

fn release_after<T>(result: Result<T, Error>, close: Result<(), Error>) -> Result<T, Error> {
    match (result, close) {
        (result, Ok(())) => result,
        (Ok(_), Err(close)) => Err(close),
        (Err(error), Err(close)) => Err(Error(format!("{error}; {close}"))),
    }
}

struct FileOwner {
    file: Option<ManuallyDrop<File>>,
    #[cfg(test)]
    close: fn(RawHandle) -> io::Result<()>,
}

impl FileOwner {
    fn open(executable: &Path) -> Result<Self, Error> {
        // Adopt the successful open before any fallible metadata/snapshot operation.
        Ok(Self {
            file: Some(ManuallyDrop::new(native::open_executable_lease(
                executable,
            )?)),
            #[cfg(test)]
            close: close_handle,
        })
    }

    fn file(&self) -> &File {
        match &self.file {
            Some(file) => file,
            None => unreachable!("trusted executable owner is already disarmed"),
        }
    }

    fn release_on_worker(mut self) -> Result<(), Error> {
        self.close()
    }

    fn close(&mut self) -> Result<(), Error> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        // Disarm the ordinary owner before calling native close. The pending guard
        // retains the File on any error/unwind, including a close of unknown outcome.
        let mut pending = PendingClose { file: Some(file) };
        #[cfg(test)]
        let close = self.close;
        #[cfg(not(test))]
        let close = close_handle;
        close(pending.file().as_raw_handle()).map_err(|error| {
            Error(format!(
                "Trusted executable handle close is unconfirmed: {error}"
            ))
        })?;
        // Native close succeeded. Never run File::drop on this closed handle.
        let _ = pending.file.take();
        Ok(())
    }
}

impl Drop for FileOwner {
    fn drop(&mut self) {
        // Backend-only fallback, used only when retiring this lease is already safe.
        // An explicit attempt has already emptied the slot, so it cannot be retried.
        let _ = self.close();
    }
}

struct PendingClose {
    file: Option<ManuallyDrop<File>>,
}

impl PendingClose {
    fn file(&self) -> &File {
        match &self.file {
            Some(file) => file,
            None => unreachable!("trusted executable close is already confirmed"),
        }
    }
}

impl Drop for PendingClose {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Unknown close results must not release the owner or close a possibly
            // reused handle. Forget the actual File, which has Drop, without retry.
            std::mem::forget(ManuallyDrop::into_inner(file));
        }
    }
}

fn close_handle(handle: RawHandle) -> io::Result<()> {
    // SAFETY: the pending guard exclusively owns this native handle; its File
    // destructor is disabled before this single close attempt.
    if unsafe { CloseHandle(handle) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use super::{ExecutableLease, RawHandle, io};
    use crate::Error;
    use std::sync::atomic::Ordering;

    pub(super) fn set_probe_close(
        lease: &mut ExecutableLease,
        close: fn(RawHandle) -> io::Result<()>,
    ) {
        lease.probe_close = close;
    }

    pub(super) fn set_held_close(
        lease: &mut ExecutableLease,
        close: fn(RawHandle) -> io::Result<()>,
    ) {
        lease.owner.close = close;
    }

    pub(super) fn probe_opens(lease: &ExecutableLease) -> usize {
        lease.probe_opens.load(Ordering::Relaxed)
    }

    pub(super) fn raw_handle(lease: &ExecutableLease) -> RawHandle {
        use std::os::windows::io::AsRawHandle;
        lease.owner.file().as_raw_handle()
    }

    pub(super) fn close_with(
        mut lease: ExecutableLease,
        close: fn(RawHandle) -> io::Result<()>,
    ) -> Result<(), Error> {
        lease.owner.close = close;
        lease.release_on_worker()
    }

    pub(super) fn drop_with(mut lease: ExecutableLease, close: fn(RawHandle) -> io::Result<()>) {
        lease.owner.close = close;
        drop(lease);
    }
}

#[cfg(test)]
#[path = "trusted_executable_windows_tests.rs"]
mod tests;
