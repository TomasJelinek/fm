//! Startup-reserved cleanup resources; no provider values enter fallible admission.
use crate::{retirement::*, worker_retirement::ActorRetirement};
use std::io;

/// Reserved engine cleanup capacity, established before accepting provider ownership.
///
/// Create with [`crate::Engine::lifetime`]. Plain cleanup and process-tracking workers
/// remain alive while admitted owners exist, independently of Tokio. Drop never joins.
pub struct EngineLifetime {
    pub(crate) run: RetirementLease,
    pub(crate) services: RetirementLease,
    pub(crate) commands: RetirementLease,
    pub(crate) resources: Resources,
}
pub(crate) struct Resources {
    pub(crate) operations: ActorRetirement,
    pub(crate) copies: ActorRetirement,
    pub(crate) drag: DragRetirement,
    pub(crate) snapshots: RetirementLease,
    pub(crate) checksums: crate::checksum_actor::Retirement,
    pub(crate) user_commands: crate::user_commands::Retirement,
    pub(crate) user_command_processes: crate::user_command_runtime::ProcessTracker,
}
pub(crate) struct DragRetirement {
    pub(crate) state: RetirementLease,
    pub(crate) pending: RetirementLease,
    pub(crate) backlog: RetirementWaiter,
}
impl EngineLifetime {
    pub(crate) fn new() -> io::Result<Self> {
        // Copy and mutation each retain 32 jobs, one intake and one service context.
        // Drag retains state, pending drops and one serial backlog drain.
        // The run, shared services, command receiver and snapshots each retain one slot.
        let capacity = crate::operations::MAX_JOBS
            + crate::copy_operations::MAX_JOBS
            + 11
            + crate::checksum_actor::RETIREMENT_CAPACITY
            + crate::user_commands::MAX_JOBS
            + 1;
        let domain = RetirementDomain::try_start(capacity, 4)?;
        // No native child exists until this separate fixed process tracker is ready.
        let user_command_processes = crate::user_command_runtime::ProcessTracker::try_start()?;
        let actor = || -> io::Result<ActorRetirement> {
            Ok(ActorRetirement {
                domain: domain.clone(),
                intake: domain.new_waiter().map_err(io::Error::other)?,
                service: domain.try_reserve().map_err(io::Error::other)?,
            })
        };
        Ok(Self {
            run: domain.try_reserve().map_err(io::Error::other)?,
            services: domain.try_reserve().map_err(io::Error::other)?,
            commands: domain.try_reserve().map_err(io::Error::other)?,
            resources: Resources {
                operations: actor()?,
                copies: actor()?,
                drag: DragRetirement {
                    state: domain.try_reserve().map_err(io::Error::other)?,
                    pending: domain.try_reserve().map_err(io::Error::other)?,
                    backlog: domain.new_waiter().map_err(io::Error::other)?,
                },
                snapshots: domain.try_reserve().map_err(io::Error::other)?,
                checksums: crate::checksum_actor::Retirement {
                    domain: domain.clone(),
                    intake: domain.new_waiter().map_err(io::Error::other)?,
                    service: domain.try_reserve().map_err(io::Error::other)?,
                },
                user_commands: crate::user_commands::Retirement {
                    domain: domain.clone(),
                    service: domain.try_reserve().map_err(io::Error::other)?,
                },
                user_command_processes,
            },
        })
    }
}
#[cfg(test)]
impl DragRetirement {
    pub(crate) fn testing() -> Self {
        let domain = RetirementDomain::try_start(3, 1).unwrap();
        Self {
            state: domain.try_reserve().unwrap(),
            pending: domain.try_reserve().unwrap(),
            backlog: domain.new_waiter().unwrap(),
        }
    }
}
