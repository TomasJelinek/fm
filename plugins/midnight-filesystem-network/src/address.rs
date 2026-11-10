use crate::{
    adapter::{NodeKind, Record, Target},
    api::{Address, AddressEncoding, ErrorCode, Failure},
};
use std::collections::HashSet;

pub const MAX_DEPTH: usize = 64;
pub const MAX_ADDRESS_BYTES: usize = 16 * 1024 * 1024;
const PREFIX: &[u8; 8] = b"NWFS\x01\0\0\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub trail: Vec<Record>,
}

impl Location {
    pub fn encode(&self) -> Result<Address, Failure> {
        let size = self.validate()?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| limit())?;
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(&(self.trail.len() as u32).to_le_bytes());
        for record in &self.trail {
            bytes.push(record.target.kind as u8);
            for value in [record.target.key.as_slice(), record.display.as_bytes()] {
                bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
                bytes.extend_from_slice(value);
            }
        }
        Ok(Address {
            encoding: AddressEncoding::Opaque,
            bytes,
        })
    }
    pub fn decode(address: &Address) -> Result<Self, Failure> {
        if address.bytes.len() > MAX_ADDRESS_BYTES {
            return Err(limit());
        }
        if address.encoding != AddressEncoding::Opaque || !address.bytes.starts_with(PREFIX) {
            return Err(invalid());
        }
        let mut reader = Reader(&address.bytes[PREFIX.len()..]);
        let count = reader.length()?;
        if count > MAX_DEPTH {
            return Err(limit());
        }
        let mut trail = Vec::new();
        trail.try_reserve_exact(count).map_err(|_| limit())?;
        for _ in 0..count {
            let kind = match reader.take(1)?[0] {
                0 => NodeKind::Root,
                1 => NodeKind::Container,
                2 => NodeKind::Server,
                3 => NodeKind::Share,
                _ => return Err(invalid()),
            };
            let key = reader.blob()?;
            let display = String::from_utf8(reader.blob()?).map_err(|_| invalid())?;
            trail.push(Record {
                target: Target { kind, key },
                display,
            });
        }
        if !reader.0.is_empty() {
            return Err(invalid());
        }
        let location = Self { trail };
        location.validate()?;
        Ok(location)
    }
    pub fn parent(&self) -> Option<Self> {
        (self.trail.len() > 1).then(|| Self {
            trail: self.trail[..self.trail.len() - 1].to_vec(),
        })
    }
    fn validate(&self) -> Result<usize, Failure> {
        if self.trail.len() > MAX_DEPTH {
            return Err(limit());
        }
        if self
            .trail
            .first()
            .is_none_or(|root| root.target.kind != NodeKind::Root)
        {
            return Err(invalid());
        }
        let mut seen = HashSet::new();
        seen.try_reserve(self.trail.len()).map_err(|_| limit())?;
        let mut size = 12_usize;
        for (index, record) in self.trail.iter().enumerate() {
            if !seen.insert(&record.target)
                || (index != 0 && record.target.kind == NodeKind::Root)
                || (index + 1 != self.trail.len() && record.target.kind == NodeKind::Share)
            {
                return Err(invalid());
            }
            size = size
                .checked_add(9)
                .and_then(|size| size.checked_add(record.target.key.len()))
                .and_then(|size| size.checked_add(record.display.len()))
                .filter(|size| *size <= MAX_ADDRESS_BYTES)
                .ok_or_else(limit)?;
        }
        Ok(size)
    }
}

fn invalid() -> Failure {
    Failure::new(ErrorCode::Invalid, "Invalid saved network location.")
}
fn limit() -> Failure {
    Failure::new(
        ErrorCode::ResourceLimit,
        "Network address resource budget exceeded.",
    )
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], Failure> {
        let Some((value, rest)) = self.0.split_at_checked(length) else {
            return Err(invalid());
        };
        self.0 = rest;
        Ok(value)
    }
    fn length(&mut self) -> Result<usize, Failure> {
        let value: [u8; 4] = self.take(4)?.try_into().map_err(|_| invalid())?;
        Ok(u32::from_le_bytes(value) as usize)
    }
    fn blob(&mut self) -> Result<Vec<u8>, Failure> {
        let length = self.length()?;
        let bytes = self.take(length)?;
        let mut value = Vec::new();
        value.try_reserve_exact(bytes.len()).map_err(|_| limit())?;
        value.extend_from_slice(bytes);
        Ok(value)
    }
}

#[cfg(test)]
#[path = "address_tests.rs"]
mod tests;
