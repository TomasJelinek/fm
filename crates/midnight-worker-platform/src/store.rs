//! OS-owned private artifact storage. Only validated hexadecimal cache basenames are accepted.
use std::{
    fs::{File, FileTimes, TryLockError},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant, SystemTime},
};

#[cfg(windows)]
#[path = "store_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "store_linux.rs"]
mod native;

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
impl Transaction {
    /// Invalid or interrupted generation writes require a fresh directory snapshot.
    pub fn generation(&self) -> Option<u64> {
        self.generation
    }
    /// Call before any mutation. A failed transaction must not publish its index generation.
    pub fn advance(&mut self) -> io::Result<()> {
        let Some(generation) = self.generation else {
            return Ok(());
        };
        let next = generation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Cache generation exhausted"))?;
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
impl PrivateStore {
    pub fn open(path: &Path) -> io::Result<Self> {
        native::Root::open(path).map(Self)
    }
    /// Backend only. The persistent lock inode is never renamed or removed.
    pub fn transaction(&self) -> io::Result<Transaction> {
        let (mut file, created) = self.0.lock_file()?;
        let deadline = Instant::now() + Duration::from_secs(1);
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
                        transaction.write_generation(0)?;
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
