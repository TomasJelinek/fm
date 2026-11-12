use crate::Limits;
use std::io;

/// Explicit total CPU-time policy, separate from host idle deadlines and cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuBudget {
    /// Keep the legacy one-to-sixty second aggregate process CPU limit.
    Seconds(u32),
    /// Add no aggregate CPU ceiling. Inherited OS limits remain in force; memory,
    /// sandboxing and host cancellation still apply. This never raises an OS limit.
    NoAdditionalLimit,
}

/// Opt-in resource policy for the version2 startup handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourcePolicy {
    pub memory_bytes: u64,
    pub cpu: CpuBudget,
}

impl ResourcePolicy {
    pub fn validate(self) -> io::Result<Self> {
        let valid_cpu = match self.cpu {
            CpuBudget::Seconds(seconds) => (1..=60).contains(&seconds),
            CpuBudget::NoAdditionalLimit => true,
        };
        if !(64 * 1024 * 1024..=1024 * 1024 * 1024).contains(&self.memory_bytes) || !valid_cpu {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid worker resource policy",
            ));
        }
        Ok(self)
    }
}

impl From<Limits> for ResourcePolicy {
    fn from(limits: Limits) -> Self {
        Self {
            memory_bytes: limits.memory_bytes,
            cpu: CpuBudget::Seconds(limits.cpu_seconds),
        }
    }
}
