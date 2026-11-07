//! Public startup contract for trusted executable plugins before parsing untrusted input.
//! A provider must call `enter` before any content parsing. Failure is terminal.
#![deny(unsafe_op_in_unsafe_fn)]

use std::io::{self, Read, Write};

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

/// Worker resource limits. Wall-clock deadlines and cancellation belong to the host supervisor.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub memory_bytes: u64,
    pub cpu_seconds: u32,
}

impl Limits {
    /// Validate before passing limits across the process boundary.
    pub fn validate(self) -> io::Result<Self> {
        if !(64 * 1024 * 1024..=1024 * 1024 * 1024).contains(&self.memory_bytes)
            || !(1..=60).contains(&self.cpu_seconds)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid worker limits",
            ));
        }
        Ok(self)
    }
}

/// Host startup message, sent before the provider-specific protocol.
pub fn write_limits(writer: &mut impl Write, limits: Limits) -> io::Result<()> {
    let limits = limits.validate()?;
    writer.write_all(b"MWS1")?;
    writer.write_all(&limits.memory_bytes.to_le_bytes())?;
    writer.write_all(&limits.cpu_seconds.to_le_bytes())?;
    writer.flush()
}

/// Refuse any worker that did not finish its required OS restriction phase.
pub fn read_ready(reader: &mut impl Read) -> io::Result<()> {
    let mut ready = [0; 4];
    reader.read_exact(&mut ready)?;
    if &ready != b"MWR1" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Worker sandbox unavailable",
        ));
    }
    Ok(())
}

/// Enter/verify the OS restriction and acknowledge it. Never continue decoding after an error.
pub fn enter(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<()> {
    let mut header = [0; 16];
    reader.read_exact(&mut header)?;
    if &header[..4] != b"MWS1" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid worker startup",
        ));
    }
    let limits = Limits {
        memory_bytes: u64::from_le_bytes(header[4..12].try_into().map_err(io::Error::other)?),
        cpu_seconds: u32::from_le_bytes(header[12..16].try_into().map_err(io::Error::other)?),
    }
    .validate()?;
    platform::restrict(limits)?;
    writer.write_all(b"MWR1")?;
    writer.flush()
}
