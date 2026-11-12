//! Startup-reserved cleanup resources; no provider values enter fallible admission.
use crate::{retirement::*, worker_retirement::ActorRetirement};
use std::io;

/// Reserved engine cleanup capacity, established before accepting provider ownership.
///
/// Create with [`crate::Engine::lifetime`]. One plain worker remains alive while
/// admitted owners exist, independently of Tokio. Dropping this token never joins it.
pub struct EngineLifetime {
    pub(crate) run: RetirementLease,
    pub(crate) commands: RetirementLease,
    pub(crate) resources: Resources,
}
pub(crate) struct Resources {
    pub(crate) operations: ActorRetirement,
    pub(crate) copies: ActorRetirement,
    pub(crate) drag: DragRetirement,
    pub(crate) snapshots: RetirementLease,
}
pub(crate) struct DragRetirement {
    pub(crate) state: RetirementLease,
    pub(crate) pending: RetirementLease,
    pub(crate) backlog: RetirementWaiter,
}
impl EngineLifetime {
    pub(crate) fn new() -> io::Result<Self> {
        // Each actor retains 32 accepted jobs, one serialized intake and one service
        // context. Drag retains state, pending drops and one serial backlog drain.
        // The run bundle, command receiver and panel snapshots each retain one slot.
        let capacity = crate::operations::MAX_JOBS + crate::copy_operations::MAX_JOBS + 10;
        let domain = RetirementDomain::try_start(capacity, 3)?;
        let actor = || -> io::Result<ActorRetirement> {
            Ok(ActorRetirement {
                domain: domain.clone(),
                intake: domain.new_waiter().map_err(io::Error::other)?,
                service: domain.try_reserve().map_err(io::Error::other)?,
            })
        };
        Ok(Self {
            run: domain.try_reserve().map_err(io::Error::other)?,
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
