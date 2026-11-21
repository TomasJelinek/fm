//! OS-owned private artifact storage. Only validated hexadecimal cache basenames are accepted.
use std::{
    fs::{File, FileTimes, TryLockError},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant, SystemTime},
};

#[cfg(windows)]
#[path = "store_windows.rs"]
pub(crate) mod native;
#[cfg(target_os = "linux")]
#[path = "store_linux.rs"]
pub(crate) mod native;

pub struct Record {
    pub name: String,
    pub bytes: u64,
    pub modified: SystemTime,
}

pub struct PrivateStore(native::Root);
/// An exclusive transaction across independent cache instances and processes.
pub struct Transaction {
    file: File,
    generation: Option<u64>,
}
/// A generation no cache instance can have remembered: randomly keyed per call.
fn fresh_generation() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
    );
    hasher.write_u32(std::process::id());
    hasher.finish()
}
impl Transaction {
    /// Invalid or interrupted generation writes require a fresh directory snapshot.
    pub fn generation(&self) -> Option<u64> {
        self.generation
    }
    /// Call before any mutation. A failed transaction must not publish its index generation.
    /// A missing or torn record (for example after a crash between its two writes) is
    /// repaired with a fresh random generation, so later transactions can synchronize again
    /// and no instance can mistake a value it remembers for the repaired one.
    pub fn advance(&mut self) -> io::Result<()> {
        let next = match self.generation {
            Some(generation) => generation.wrapping_add(1),
            None => fresh_generation(),
        };
        self.write_generation(next)
    }
    fn write_generation(&mut self, value: u64) -> io::Result<()> {
        self.generation = None;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&value.to_le_bytes())?;
        self.file.write_all(&(!value).to_le_bytes())?;
        self.file.set_len(16)?;
        self.generation = Some(value);
        Ok(())
    }
}
impl Drop for Transaction {
    /// Unlocks explicitly rather than by closing the lock file: a child process that another
    /// thread is starting at that moment shares the process's open files until it executes
    /// its program, and the lock belongs to the open file, so closing this handle alone
    /// would leave the store locked until then.
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
impl PrivateStore {
    pub fn open(path: &Path) -> io::Result<Self> {
        native::Root::open(path).map(Self)
    }
    /// Open one private cache bucket without traversing links or reparse points.
    /// Only two lowercase hexadecimal characters are accepted. A read-only probe
    /// never creates a directory; callers retain the parent store's transaction.
    pub fn bucket(&self, prefix: &str, create: bool) -> io::Result<Option<Self>> {
        if prefix.len() != 2
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid cache bucket",
            ));
        }
        self.0.bucket(prefix, create).map(|root| root.map(Self))
    }
    /// Backend only. The persistent lock inode is never renamed or removed. Waits up to one
    /// second for another instance or process, then reports `WouldBlock`.
    pub fn transaction(&self) -> io::Result<Transaction> {
        self.transaction_within(Duration::from_secs(1))
    }
    /// Like [`Self::transaction`], but reports `WouldBlock` at once when the store is busy.
    pub fn try_transaction(&self) -> io::Result<Transaction> {
        self.transaction_within(Duration::ZERO)
    }
    fn transaction_within(&self, wait: Duration) -> io::Result<Transaction> {
        let (mut file, created) = self.0.lock_file()?;
        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    let mut bytes = [0; 16];
                    let generation = if file.metadata()?.len() == 16
                        && file.read_exact(&mut bytes).is_ok()
                    {
                        let value = u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes"));
                        let check = u64::from_le_bytes(bytes[8..].try_into().expect("eight bytes"));
                        (value == !check).then_some(value)
                    } else {
                        None
                    };
                    let mut transaction = Transaction { file, generation };
                    if created {
                        // A recreated store must not repeat a value an instance remembers.
                        transaction.write_generation(fresh_generation())?;
                    }
                    return Ok(transaction);
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "Preview cache is busy",
                    ));
                }
                Err(TryLockError::Error(error)) => return Err(error),
            }
        }
    }
    pub fn read(&self, name: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
        check_name(name)?;
        let mut file = match self.0.read(name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        if length > limit as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Cached artifact exceeds byte limit",
            ));
        }
        let mut bytes = Vec::with_capacity(length as usize);
        (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Cached artifact grew beyond limit",
            ));
        }
        let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
        Ok(Some(bytes))
    }
    pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        check_name(name)?;
        let mut temporary = self.0.temporary()?;
        temporary.file.write_all(bytes)?;
        temporary.file.sync_all()?;
        self.0.publish(temporary, name)
    }
    pub fn touch(&self, name: &str) -> io::Result<()> {
        check_name(name)?;
        self.0
            .read(name)?
            .set_times(FileTimes::new().set_modified(SystemTime::now()))
    }
    pub fn records(&self) -> io::Result<Vec<Record>> {
        self.0.records()
    }
    /// Inspect one validated ordinary artifact without reading its payload.
    pub fn record(&self, name: &str) -> io::Result<Option<Record>> {
        check_name(name)?;
        let file = match self.0.read(name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        Ok(Some(Record {
            name: name.into(),
            bytes: metadata.len(),
            modified: metadata.modified()?,
        }))
    }
    pub fn remove(&self, name: &str) -> io::Result<()> {
        check_name(name)?;
        self.0.remove(name)
    }
}

fn check_name(name: &str) -> io::Result<()> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid cache basename",
        ))
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    name.len() == 68
        && name.ends_with(".mtc")
        && name.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ended_transaction_releases_the_store_while_a_copy_of_its_lock_file_stays_open() {
        // A copy of the open lock file stands in for a child process that another thread
        // starts during the transaction: it shares the lock file until it executes.
        // As the integration tests: a root whose path has no link or reparse point.
        let root = match std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT") {
            Some(root) => tempfile::tempdir_in(root).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let store = PrivateStore::open(&root.path().join("cache")).unwrap();
        let transaction = store.transaction().unwrap();
        let copy = transaction.file.try_clone().unwrap();
        drop(transaction);
        assert!(store.try_transaction().is_ok());
        drop(copy);
    }
}
