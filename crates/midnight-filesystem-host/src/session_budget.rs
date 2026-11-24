//! What one exchange with an archive worker may cost the host, and how long it may last.
//!
//! Every exchange, of every protocol, has three bounds beside the idle-progress watchdog
//! (`DEADLINE`, renewed by every completed byte I/O, source reads included):
//!
//! - the bytes the host serves the worker (`SourceData` and `SpoolData`) in that one
//!   exchange (`Budget::served`);
//! - a wall-clock time budget (`Budget::time`) that only the worker's progress replies
//!   renew (an acknowledged `StageAppend` or `StageProgress`), never a read it requests;
//! - the cancellation of its caller where the caller may abandon it (`Hard`).
use midnight_core::Cancellation;
use midnight_filesystem_api::Protocol;
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;

/// The bounds of one mount's exchanges. Production uses `Budget::DEFAULT`; tests use
/// small values to reach every bound quickly.
#[derive(Clone, Copy, Debug)]
pub(super) struct Budget {
    /// Bytes served per byte of the source and of the sealed scratch stages.
    pub(super) served_per_byte: u64,
    /// Bytes served beyond that, for small archives and metadata probes.
    pub(super) served_extra: u64,
    /// Time budget of an exchange, beside `time_rate`.
    pub(super) time_base: Duration,
    /// Source and scratch bytes per second that the time budget grants beyond its base.
    pub(super) time_rate: u64,
    /// How long a cancelled listing or unlock may still finish before its worker is
    /// stopped.
    pub(super) cancel_grace: Duration,
}

impl Budget {
    /// The bundled worker reads its source through a window that keeps the bytes it
    /// fetched, so one exchange reads each range of its source about once: a ZIP's
    /// central directory and the local headers it probes, a TAR header walk, a
    /// compressed stream decoded once into scratch, a 7z block decoded once. Four times
    /// the source and scratch, plus 1 GiB, leaves room for re-reads; a worker that
    /// requests reads in a loop reaches it within seconds at pipe speed.
    ///
    /// Time: 10 minutes, plus one second per 2 MiB of source and scratch. Opening a
    /// compressed TAR decodes its whole source in one exchange without a progress reply
    /// of the listing kind; bzip2, the slowest bundled decoder, reads several MiB of
    /// compressed input per second, so a 10 GiB archive gets about 95 minutes. Staging
    /// a member renews the budget with every acknowledged append or progress report.
    pub(super) const DEFAULT: Self = Self {
        served_per_byte: 4,
        served_extra: 1024 * MIB,
        time_base: Duration::from_secs(10 * 60),
        time_rate: 2 * MIB,
        cancel_grace: Duration::from_secs(2),
    };

    /// The most bytes one exchange may be served. Protocol 1 keeps its original budget,
    /// the source's length.
    pub(super) fn served(&self, protocol: Protocol, source: u64, scratch: u64) -> u64 {
        if protocol == Protocol::V1 {
            return source;
        }
        source
            .saturating_add(scratch)
            .saturating_mul(self.served_per_byte)
            .saturating_add(self.served_extra)
    }

    /// The time budget of one exchange over `bytes` of source and scratch.
    pub(super) fn time(&self, bytes: u64) -> Duration {
        self.time_base
            .saturating_add(Duration::from_secs(bytes / self.time_rate.max(1)))
    }
}

/// A duration for a message: whole minutes, seconds or milliseconds.
pub(super) fn describe_time(time: Duration) -> String {
    let seconds = time.as_secs();
    if seconds >= 120 && seconds.is_multiple_of(60) {
        format!("{} minutes", seconds / 60)
    } else if seconds == 1 && time.subsec_nanos() == 0 {
        "1 second".into()
    } else if seconds > 1 && time.subsec_nanos() == 0 {
        format!("{seconds} seconds")
    } else {
        format!("{} ms", time.as_millis())
    }
}

/// A byte count for a message: whole MiB, or bytes.
pub(super) fn describe_bytes(bytes: u64) -> String {
    if bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// Cancellation that ends an exchange by stopping its worker.
#[derive(Clone, Copy)]
pub(super) enum Hard<'a> {
    /// Startup: its caller owns the whole mount, and cancelling ends the worker at once.
    Startup(&'a Cancellation),
    /// A listing or an unlock of a mount that others may share. A cancelled exchange
    /// may still finish within `Budget::cancel_grace`, and the caller discards its
    /// result; one that does not is ended by stopping the worker, which retires the
    /// mount (it is opened again).
    Request(&'a Cancellation),
}

impl<'a> Hard<'a> {
    pub(super) fn token(self) -> &'a Cancellation {
        match self {
            Self::Startup(token) | Self::Request(token) => token,
        }
    }
    /// Whether the exchange ends now: only a cancelled startup does.
    pub(super) fn ends_now(self) -> bool {
        matches!(self, Self::Startup(token) if token.is_cancelled())
    }
    pub(super) fn watched(self, grace: Duration) -> Abort {
        Abort {
            token: self.token().clone(),
            grace: match self {
                Self::Startup(_) => None,
                Self::Request(_) => Some(grace),
            },
            since: None,
        }
    }
}

/// A hard request token has no wake signal; the supervisor observes it this often
/// while such an exchange is active. Idle mounts and token-free exchanges never poll.
const STARTUP_POLL: Duration = Duration::from_millis(10);
/// A cancelled listing or unlock has a grace of seconds, so its token is observed less
/// often.
const REQUEST_POLL: Duration = Duration::from_millis(50);

/// The cancellation that the supervisor of an exchange observes (`Hard`).
pub(super) struct Abort {
    token: Cancellation,
    /// None: the exchange ends when the token is cancelled.
    grace: Option<Duration>,
    /// When the supervisor first observed the token cancelled.
    since: Option<Instant>,
}

/// Why the supervisor ends an active exchange.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Expiry {
    /// A cancelled startup.
    Cancelled,
    /// A cancelled listing or unlock that did not end within its grace.
    Abandoned(Duration),
    /// No progress before the idle deadline.
    Idle,
    /// The time budget ran out.
    Time(Duration),
}

/// The deadlines of the one active exchange.
pub(super) struct Watch {
    /// Idle deadline: every completed byte I/O renews it, source reads included.
    pub(super) idle: Instant,
    /// End of the time budget: only the worker's progress replies renew it.
    pub(super) budget: Instant,
    /// The time budget, which a progress reply grants again.
    pub(super) allowance: Duration,
    pub(super) abort: Option<Abort>,
}

impl Watch {
    pub(super) fn new(
        now: Instant,
        idle: Duration,
        allowance: Duration,
        abort: Option<Abort>,
    ) -> Self {
        Self {
            idle: now + idle,
            budget: now + allowance,
            allowance,
            abort,
        }
    }

    /// How long the supervisor may wait before it checks again, or why it ends the
    /// exchange now.
    pub(super) fn remaining(&mut self, now: Instant) -> Result<Duration, Expiry> {
        let mut until = self.idle.min(self.budget);
        let mut poll = None;
        if let Some(abort) = self.abort.as_mut() {
            if abort.since.is_none() && abort.token.is_cancelled() {
                abort.since = Some(now);
            }
            match (abort.since, abort.grace) {
                (Some(_), None) => return Err(Expiry::Cancelled),
                (Some(since), Some(grace)) => {
                    let end = since + grace;
                    if now >= end {
                        return Err(Expiry::Abandoned(grace));
                    }
                    until = until.min(end);
                }
                (None, None) => poll = Some(STARTUP_POLL),
                (None, Some(_)) => poll = Some(REQUEST_POLL),
            }
        }
        if now >= self.idle {
            return Err(Expiry::Idle);
        }
        if now >= self.budget {
            return Err(Expiry::Time(self.allowance));
        }
        let remaining = until - now;
        Ok(poll.map_or(remaining, |poll| remaining.min(poll)))
    }
}
