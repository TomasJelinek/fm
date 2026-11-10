//! One cache transaction and inventory across legacy records and private hash buckets.
use midnight_worker_platform::store::{PrivateStore, Record as NativeRecord, Transaction};
use std::{collections::HashMap, io, path::Path, time::SystemTime};

pub(super) struct Record {
    pub name: String,
    pub bytes: u64,
    pub modified: SystemTime,
    /// Both physical copies count until an explicit write or removal retires the legacy one.
    pub copies: usize,
}

pub(super) struct Store {
    root: PrivateStore,
}

impl Store {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            root: PrivateStore::open(path)?,
        })
    }

    pub fn transaction(&self) -> io::Result<Transaction> {
        self.root.transaction()
    }

    pub fn read(&self, name: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let prefix = prefix(name)?;
        if let Some(bucket) = self.root.bucket(prefix, false)?
            && let Some(bytes) = bucket.read(name, limit)?
        {
            return Ok(Some(bytes));
        }
        // A missing shard may use an existing flat v1 record. Invalid or unreadable
        // shards fail closed; they never silently reveal an older duplicate.
        self.root.read(name, limit)
    }

    pub fn touch(&self, name: &str) -> io::Result<()> {
        let prefix = prefix(name)?;
        if let Some(bucket) = self.root.bucket(prefix, false)? {
            match bucket.touch(name) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        self.root.touch(name)
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let bucket = self
            .root
            .bucket(prefix(name)?, true)?
            .ok_or_else(|| io::Error::other("Cache bucket was not created"))?;
        bucket.write(name, bytes)?;
        // Publication is atomic in the shard. Only then retire a pre-existing flat
        // record; interrupted cleanup remains accounted as two physical records.
        remove_if_present(&self.root, name)
    }

    pub fn record(&self, name: &str) -> io::Result<Option<Record>> {
        let prefix = prefix(name)?;
        let mut record = self.root.record(name)?.map(Record::from);
        if let Some(bucket) = self.root.bucket(prefix, false)?
            && let Some(sharded) = bucket.record(name)?
        {
            if let Some(record) = &mut record {
                record.add(sharded)?;
            } else {
                record = Some(sharded.into());
            }
        }
        Ok(record)
    }

    pub fn records(&self) -> io::Result<Vec<Record>> {
        let mut records = HashMap::<String, Record>::new();
        let mut count = 0usize;
        let mut append = |record: NativeRecord, bucket: Option<&str>| -> io::Result<()> {
            let actual =
                prefix(&record.name).map_err(|_| invalid("Noncanonical thumbnail cache name"))?;
            if bucket.is_some_and(|bucket| bucket != actual) {
                return Err(invalid("Thumbnail record is in the wrong hash bucket"));
            }
            count += 1;
            if count > 100_000 {
                return Err(invalid("Aggregate thumbnail cache entry limit exceeded"));
            }
            if let Some(previous) = records.get_mut(&record.name) {
                previous.add(record)?;
            } else {
                records.insert(record.name.clone(), record.into());
            }
            Ok(())
        };
        for record in self.root.records()? {
            append(record, None)?;
        }
        for index in 0..=255 {
            let prefix = format!("{index:02x}");
            if let Some(bucket) = self.root.bucket(&prefix, false)? {
                for record in bucket.records()? {
                    append(record, Some(&prefix))?;
                }
            }
        }
        Ok(records.into_values().collect())
    }

    pub fn remove(&self, name: &str) -> io::Result<()> {
        if let Some(bucket) = self.root.bucket(prefix(name)?, false)? {
            remove_if_present(&bucket, name)?;
        }
        remove_if_present(&self.root, name)
    }
}

impl From<NativeRecord> for Record {
    fn from(record: NativeRecord) -> Self {
        Self {
            name: record.name,
            bytes: record.bytes,
            modified: record.modified,
            copies: 1,
        }
    }
}

impl Record {
    fn add(&mut self, record: NativeRecord) -> io::Result<()> {
        self.bytes = self
            .bytes
            .checked_add(record.bytes)
            .ok_or_else(|| invalid("Thumbnail cache byte total overflow"))?;
        self.copies += 1;
        self.modified = self.modified.max(record.modified);
        Ok(())
    }
}

fn prefix(name: &str) -> io::Result<&str> {
    if name.len() != 68
        || !name.ends_with(".mtc")
        || !name.as_bytes()[..64]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid thumbnail cache name",
        ));
    }
    Ok(&name[..2])
}

fn remove_if_present(store: &PrivateStore, name: &str) -> io::Result<()> {
    match store.remove(name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "cache_store_tests.rs"]
mod tests;
