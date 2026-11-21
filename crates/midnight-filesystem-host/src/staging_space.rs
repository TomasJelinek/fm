//! Spool space reserved before a member is staged. A member whose declared length needs
//! a spool file (an archive inside an archive, and any other member read) claims that
//! many bytes of the spool volume's free space above its reserve before the worker writes
//! its first byte. A member that would not fit is refused at once with the space it needs
//! and the space there is, rather than after staging as much of it as fits. Claims of
//! stages still writing count against every later one, so stages that run at the same
//! time cannot promise each other the same free space; writes by other programs are not
//! counted, as for the reserve itself.
use super::{SPACE_CHECK_INTERVAL, SpoolSpace, free_space_reserve};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Bytes promised to the member stages of every mount that stages in one spool root and
/// not written by them yet. Mounts that share the process's spool root share its claims.
#[derive(Debug, Default)]
pub(crate) struct Claims(AtomicU64);

/// One member stage's reserved space: the declared bytes it has not written yet. Dropping
/// it (the stage sealed, failed or was abandoned) returns what it did not write.
#[derive(Debug)]
pub(crate) struct Claim {
    claims: Arc<Claims>,
    remaining: u64,
}

/// A member stage admitted with its claim, and the bytes it may write before it queries
/// the spool volume again (`staging::space_budget`).
#[derive(Debug)]
pub(crate) struct Admission {
    pub(crate) claim: Claim,
    pub(crate) budget: u64,
}

impl Claims {
    /// Claim `length` bytes of `space`, beside every other claim, if they fit above the
    /// volume's free-space reserve; otherwise say why not.
    pub(crate) fn admit(self: &Arc<Self>, length: u64, space: SpoolSpace) -> io::Result<Admission> {
        let free = above_reserve(space);
        let mut others = 0;
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |claimed| {
                others = claimed;
                claimed.checked_add(length).filter(|total| *total <= free)
            })
            .map_err(|_| refusal(length, free, others))?;
        Ok(Admission {
            claim: Claim {
                claims: self.clone(),
                remaining: length,
            },
            // At least `length` (or a whole check interval) is free beside the others.
            budget: free.saturating_sub(others).min(SPACE_CHECK_INTERVAL),
        })
    }

    /// Bytes promised to stages other than one whose own claim still holds `own`.
    pub(crate) fn others(&self, own: u64) -> u64 {
        self.0.load(Ordering::Acquire).saturating_sub(own)
    }

    #[cfg(test)]
    pub(crate) fn claimed(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

impl Claim {
    pub(crate) fn remaining(&self) -> u64 {
        self.remaining
    }

    /// `written` more bytes of the stage reached its spool file, where the volume's free
    /// space now counts them: they are no longer promised.
    pub(crate) fn wrote(&mut self, written: u64) {
        let released = written.min(self.remaining);
        self.remaining -= released;
        self.claims.0.fetch_sub(released, Ordering::AcqRel);
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.claims.0.fetch_sub(self.remaining, Ordering::AcqRel);
    }
}

/// Free space of the volume above its reserve: what staging may use of it at most.
pub(crate) fn above_reserve(space: SpoolSpace) -> u64 {
    space
        .available
        .saturating_sub(free_space_reserve(space.total))
}

fn refusal(length: u64, free: u64, others: u64) -> io::Error {
    let promised = if others == 0 {
        String::new()
    } else {
        format!(
            ", {} of which other archive reads in progress still need",
            amount(others)
        )
    };
    io::Error::new(
        io::ErrorKind::StorageFull,
        format!(
            "This archive member needs {} of archive spool space, but the spool volume has \
             only {} free above its reserve (the larger of 1 GiB and 2% of its capacity){}. \
             Free some space on that volume and try again.",
            amount(length),
            amount(free),
            promised
        ),
    )
}

/// `bytes` for a message: exact below 1 KiB, else in binary units with one decimal.
pub(crate) fn amount(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
