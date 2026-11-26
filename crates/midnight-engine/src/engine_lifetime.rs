//! Startup-reserved cleanup resources; no provider values enter fallible admission.
use crate::{retirement::*, worker_retirement::ActorRetirement};
use midnight_work_budget::resident;
use std::io;

/// Reserved engine cleanup and resident workers, established before accepting providers.
///
/// Create with [`crate::Engine::lifetime`]. Plain cleanup and process-tracking workers
/// remain alive while admitted owners exist, independently of Tokio. Drop never joins.
pub struct EngineLifetime {
    pub(crate) run: RetirementLease,
    pub(crate) comparator_policies: RetirementLease,
    pub(crate) services: RetirementLease,
    pub(crate) commands: RetirementLease,
    pub(crate) resources: Resources,
}
pub(crate) struct Resources {
    pub(crate) resident_owner: resident::Owner,
    pub(crate) resident: resident::Spawner,
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
    pub(crate) shell_menu: crate::shell_menu::Retirement,
}
pub(crate) struct DragRetirement {
    pub(crate) state: RetirementLease,
    pub(crate) pending: RetirementLease,
    pub(crate) backlog: RetirementWaiter,
}

/// Payload slots of the engine retirement domain. `docs/developer/ENGINE_LIFETIME.md`
/// states this total and its per-owner breakdown; a test keeps the two in step.
// Copy and mutation each retain 32 jobs, one intake and one service context.
// Drag retains state, pending drops and one serial backlog drain.
// The run, shared services, command receiver and snapshots each retain one slot.
pub(crate) const DOMAIN_CAPACITY: usize = crate::operations::MAX_JOBS
    + crate::copy_operations::MAX_JOBS
    + 11
    + 1 // Shared comparator policy store.
    + crate::checksum_actor::RETIREMENT_CAPACITY
    + crate::user_commands::MAX_JOBS
    + 1
    + crate::clipboard::RETIREMENT_CAPACITY
    + crate::desktop::EDITOR_RETIREMENT_CAPACITY
    + crate::shell_menu::RETIREMENT_CAPACITY
    + crate::watch::RETIREMENT_CAPACITY
    + crate::network_neighborhood::RETIREMENT_CAPACITY
    + crate::file_list_actor::RETIREMENT_CAPACITY
    + crate::archive_sizes::RETIREMENT_CAPACITY
    + crate::archives::RETIREMENT_CAPACITY
    + crate::report_arena::RETIREMENT_CAPACITY
    + crate::tree_rename::RETIREMENT_CAPACITY
    + crate::find::RETIREMENT_CAPACITY;
/// Reusable waiter tokens: the serialized intakes of operations, copies, archives,
/// checksums, shell menu and desktop, the drag backlog drain and the clipboard
/// rejection path.
pub(crate) const DOMAIN_WAITERS: usize = 8;
impl EngineLifetime {
    pub(crate) fn new() -> io::Result<Self> {
        // Model, presence, archive and watch lanes (2 each), drag (2),
        // clipboard (1), and report readers (2).
        let (resident_owner, resident) = resident::Owner::start("midnight-engine", 13)?;
        let domain = RetirementDomain::try_start(DOMAIN_CAPACITY, DOMAIN_WAITERS)?;
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
            comparator_policies: domain.try_reserve().map_err(io::Error::other)?,
            services: domain.try_reserve().map_err(io::Error::other)?,
            commands: domain.try_reserve().map_err(io::Error::other)?,
            resources: Resources {
                resident_owner,
                resident,
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
                shell_menu: crate::shell_menu::Retirement {
                    intake: domain.new_waiter().map_err(io::Error::other)?,
                },
                desktop: crate::desktop::Retirement {
                    intake: domain.new_waiter().map_err(io::Error::other)?,
                    external: crate::desktop_external_compare::ExternalComparisonFrontDoor::new(
                        domain.clone(),
                    ),
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

#[cfg(test)]
mod tests {
    use super::{DOMAIN_CAPACITY, DOMAIN_WAITERS};

    const LIFETIME_DOC: &str = include_str!("../../../docs/developer/ENGINE_LIFETIME.md");

    #[test]
    fn lifetime_document_states_the_domain_capacity_and_its_breakdown() {
        let words = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
        ];
        let capacity = format!("{DOMAIN_CAPACITY} payload slots");
        let waiters = format!("{} reusable waiter tokens", words[DOMAIN_WAITERS]);
        let text = LIFETIME_DOC
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains(&capacity) && text.contains(&waiters),
            "ENGINE_LIFETIME.md must document {capacity} and {waiters}"
        );

        let table = LIFETIME_DOC
            .split("| Owners | Slots |")
            .nth(1)
            .expect("capacity table");
        let rows: Vec<&str> = table
            .lines()
            .skip(2)
            .take_while(|line| line.starts_with('|'))
            .collect();
        assert!(!rows.is_empty(), "capacity table rows");
        let total: usize = rows
            .iter()
            .map(|row| {
                let slots = row.trim_end_matches('|').rsplit('|').next().unwrap_or("");
                slots
                    .trim()
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("slot count in {row:?}"))
            })
            .sum();
        assert_eq!(
            total, DOMAIN_CAPACITY,
            "the ENGINE_LIFETIME.md capacity table must add up to the domain capacity"
        );
    }
}
