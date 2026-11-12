# Make File List native ownership

Baseline b3fa92e. Approved scope: actor/output/worker guards and focused qualification.

- [x] Reproduce native conflict-pin final Drop on Cancel, generation replacement,
      shutdown and Controller Drop; confirmation aborted before first poll; complete
      unconsumed actual generated Spool disposal.
- [x] Enforce six retained logical generations/outputs, independently of four visible
      jobs and two active workers. Keep worker capacity through async delivery.
- [x] Before accepting a generation, reserve its logical token and two cleanup leases.
      Failed admission preserves the previously accepted generation and all user output.
- [x] Guard Job native pins and sizes; retain the same token inside guarded Job and
      generated Spool until actual payload destruction. Source output never owns Job.
- [x] Leave confirmed Pending inside guarded Job until native execution begins.
- [x] Anchor original services across both async and native task layers; cancel on Drop.
- [x] Keep verified replacement/append and source identity behavior unchanged.
- [x] Add retained output exhaustion and drop/unwind ordering regressions.
- [x] Complete the enclosing Windows/Linux gates, including existing output contracts.

Root integration: add 12 slots, no waiter, to the engine retirement domain;
`file_list_actor::Retirement { domain: domain.clone() }` and existing TaskLifetime
enter Controller::new(retirement, scope, service, mutation, outbox, snapshots).
Six is an explicit admission cap, not an assumption that completed outputs are
bounded by the visible job map. Each admitted generation produces at most one
Spool; confirmation reuses it. Outputs can retain their token after their window
or job disappears. Neither cleanup admission nor native quotas become reusable
until guarded payload destruction finishes, including a single unwinding Drop.
No public/plugin contract, output byte limit, per-drop thread or unbounded queue.

Implementation and review evidence: [File List retirement](../../verification/2026-11-12-file-list-retirement.md).
