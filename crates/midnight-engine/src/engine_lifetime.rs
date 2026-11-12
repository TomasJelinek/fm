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
    pub(crate) clipboard: crate::clipboard::Retirement,
    pub(crate) network: crate::network_neighborhood::Retirement,
    pub(crate) file_lists: crate::file_list_actor::Retirement,
    pub(crate) archive_sizes: crate::archive_sizes::Retirement,
    pub(crate) reports: crate::report_arena::ReportArena,
    pub(crate) tree_rename: crate::tree_rename::Retirement,
    pub(crate) find: crate::find::Retirement,
    pub(crate) archives: crate::archives::Retirement,
    pub(crate) watch: crate::watch::Retirement,
    pub(crate) checksums: crate::checksum_actor::Retirement,
    pub(crate) user_commands: crate::user_commands::Retirement,
    pub(crate) user_command_processes: crate::user_command_runtime::ProcessTracker,
    pub(crate) desktop_openers: crate::user_command_runtime::ProcessTracker,
    pub(crate) desktop: crate::desktop::Retirement,
}
pub(crate) struct DragRetirement {
    pub(crate) state: RetirementLease,
    pub(crate) pending: RetirementLease,
    pub(crate) backlog: RetirementWaiter,
}
impl EngineLifetime {
    /// Shared cleanup for diagnostic reports retained by standalone directory trees.
    ///
    /// Clone this before transferring the lifetime to an engine. Retained reports may
    /// outlive both the engine and Tokio; their native files still retire on workers.
    pub fn report_arena(&self) -> crate::ReportArena {
        self.resources.reports.clone()
    }

    pub(crate) fn new() -> io::Result<Self> {
        // Copy and mutation each retain 32 jobs, one intake and one service context.
        // Drag retains state, pending drops and one serial backlog drain.
        // The run, shared services, command receiver and snapshots each retain one slot.
        let capacity = crate::operations::MAX_JOBS
            + crate::copy_operations::MAX_JOBS
            + 11
            + crate::checksum_actor::RETIREMENT_CAPACITY
            + crate::user_commands::MAX_JOBS
            + 1
            + crate::clipboard::RETIREMENT_CAPACITY
            + crate::desktop::EDITOR_RETIREMENT_CAPACITY
            + crate::watch::RETIREMENT_CAPACITY
            + crate::network_neighborhood::RETIREMENT_CAPACITY
            + crate::file_list_actor::RETIREMENT_CAPACITY
            + crate::archive_sizes::RETIREMENT_CAPACITY
            + crate::archives::RETIREMENT_CAPACITY
            + crate::report_arena::RETIREMENT_CAPACITY
            + crate::tree_rename::RETIREMENT_CAPACITY
            + crate::find::RETIREMENT_CAPACITY;
        let domain = RetirementDomain::try_start(capacity, 7)?;
        // No native child exists until this separate fixed process tracker is ready.
        let user_command_processes = crate::user_command_runtime::ProcessTracker::try_start()?;
        let desktop_openers = crate::user_command_runtime::ProcessTracker::try_start_openers()?;
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
                watch: crate::watch::Retirement {
                    panes: [
                        domain.try_reserve().map_err(io::Error::other)?,
                        domain.try_reserve().map_err(io::Error::other)?,
                    ],
                },
                archives: crate::archives::Retirement {
                    domain: domain.clone(),
                    intake: domain.new_waiter().map_err(io::Error::other)?,
                },
                reports: crate::report_arena::ReportArena::try_start(
                    domain.try_reserve().map_err(io::Error::other)?,
                )?,
                find: crate::find::Retirement {
                    domain: domain.clone(),
                },
                tree_rename: crate::tree_rename::Retirement {
                    domain: domain.clone(),
                },
                archive_sizes: crate::archive_sizes::Retirement {
                    panes: [
                        domain.try_reserve().map_err(io::Error::other)?,
                        domain.try_reserve().map_err(io::Error::other)?,
                    ],
                },
                file_lists: crate::file_list_actor::Retirement {
                    domain: domain.clone(),
                },
                network: crate::network_neighborhood::Retirement {
                    provider: domain.try_reserve().map_err(io::Error::other)?,
                },
                clipboard: crate::clipboard::Retirement {
                    worker: domain.try_reserve().map_err(io::Error::other)?,
                    rejected: domain.new_waiter().map_err(io::Error::other)?,
                },
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
                desktop_openers,
                desktop: crate::desktop::Retirement {
                    intake: domain.new_waiter().map_err(io::Error::other)?,
                },
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
