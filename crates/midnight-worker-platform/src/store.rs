//! OS-owned private artifact storage. Only validated hexadecimal cache basenames are accepted.
use std::{
    fs::FileTimes,
    io::{self, Read, Write},
    path::Path,
    time::SystemTime,
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
impl PrivateStore {
    pub fn open(path: &Path) -> io::Result<Self> {
        native::Root::open(path).map(Self)
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
