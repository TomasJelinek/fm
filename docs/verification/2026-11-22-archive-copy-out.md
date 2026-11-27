# Archive copy-out speed and Cancel (WIN-8) — 2026-11-22

## Scope

Copying many small members out of an archive was slow on both platforms, about 81–87
files/s, and Cancel of a copy of a 100,001-member selection took 114–117 s to end the
job ([Windows acceptance record](../developer/acceptance/windows.md), WIN-8). This pass
measured each layer of a small member's copy, removed what it found, and records the
numbers before and after.

All of it is in an isolated `git archive` export of `0a6d5a4a`.
Builds used their own target directory, the debug test profile and
`CARGO_INCREMENTAL=0`. Test runs had TEMP on a volume that creates no 8.3 names. Other
agents' builds kept the machine at 30–100 % CPU throughout, so before and after runs were
interleaved and each table gives every run. All runs were Windows; Linux was not
measured.

## Layers and fixes

| Layer | Found | Fix | Where |
|---|---|---|---|
| Archive source | The copy validated the archive file 5–6 times per member, reopening it by its path each time (225–240 µs each) | It validates before a folder's single listing and before a Retry; the host still validates as each worker exchange begins and around each source read it serves | `archive_transfer.rs`, `midnight-filesystem-host` `session.rs` |
| Copy actor | One blocking-pool hop per step, 6–8 per small file | `Job::steps` batches steps for up to 5 ms (`STEP_BUDGET`). A batch also ends at every boundary the actor acts on, and after the step in flight once the actor stops awaiting it (`Awaited`) | `copy_worker.rs`, `copy_operations.rs` |
| Namespace (Windows) | Two turns per file (stage creation, publication), each checking the destination's whole ancestry level by level | One private `.midnight-stage-*` folder per receiving destination folder: stages take no turn, and each publication is one turn. A qualified (local NTFS, no 8.3 names) held folder proves its ancestry by handle | `transfer_destination_staging.rs`, `transfer_destination_security.rs`, `transfer_namespace_parent.rs` |
| Completion notices | One `RootCompleted` event per root; 4,098 of them could queue behind a UI that was refreshing | `RootsCompleted` joins the roots that wait into notices of at most 512, still in order and before the terminal progress | `copy_operation.rs`, `copy_delivery.rs` |
| Deselection | `archive_selection::apply` copied the whole selection and recounted its totals for every 64 copied members | An unshared selection changes in place in the panel (at most 4,096 per pass); the backend copies a shared one once, then changes it in place; totals are adjusted member by member | `copy_selection.rs`, `archive_selection.rs` |

## Measurements

### Copy actor, 2,001 small members of an in-memory archive

`archive_copy_rate_through_the_copy_actor`, `MIDNIGHT_PERF_FILES=2001`, `--ignored
--nocapture --test-threads 1`. Before is the export with only the probes and the archive
source fix (`engine-v4`; an in-memory archive validates in microseconds, so that fix does
not change this test). After is the export with every fix and the probes. Runs are listed
in their order.

| Run | Folder files/s | Selection files/s |
|---|---|---|
| before (earlier the same day) | 72 | 70 |
| after, 1 | 198 | 146 |
| before | 63 | 62 |
| after, 2 | 126 | 118 |

Per file, before → after (the ranges cover the runs above):

| Probe | Before | After |
|---|---|---|
| blocking-pool hops | 8.00 per file | 1.00 per file |
| hop overhead (hop time minus step time) | about 2.0 ms | 0.3–0.4 ms |
| namespace turns | 2 per file | 1 per file |
| namespace operation + finish | 11.3–11.6 ms | 2.7–5.0 ms |
| stage creation | 4.7–5.1 ms | 0.56–0.64 ms |
| pin refreshes | 5–9 per file, 2.9 ms | 2–6 per file, 0.5–1.0 ms |

### Archive source with the installed archive worker

`archive_copy_rate_installed_worker` (job level, no copy actor), 10,001 members of
`flat10k.tar` on D:, `MIDNIGHT_PERF_CAPTURE=native`, run alternately with the base
harness (`engine-v1`) and the final probed build (`engine-v9`):

| Mode | Before | After |
|---|---|---|
| folder | 218 files/s, 4.58 ms per file; 6 archive validations per file, 1.43 ms; read exchange 1.06 ms | 449 files/s, 2.23 ms per file; no validation per file; read exchange 0.49 ms |
| selection | 243 files/s, 4.12 ms per file; 5 validations per file, 1.17 ms; read exchange 1.01 ms | 448 files/s, 2.23 ms per file; no validation per file; read exchange 0.48 ms |

### Windows namespace pieces

- Staging-folder descriptor: 20–29 µs per staged file, or 34–35 µs in an
  auto-inherited folder (`descriptor_cost_per_staged_file`, 2,000 files).
- Handle proof of a 9-level ancestry: 114–133 µs, against 298–351 µs level by level
  (`measure_final_path_against_ancestry_opens`).
- In the copy actor, the publication's `held binding` went from 1.5 to about 1.05 ms.

### Final debug build (probes removed)

| Measurement | Result |
|---|---|
| `archive_copy_rate_through_the_copy_actor`, 2,001 members | folder 155 files/s, selection 183 files/s |
| `archive_copy_pause_and_cancel_latency`, 3,000 members | paused 0.1 ms after the request, no file written in the next 500 ms; Cancel ended the job 221.5 ms after the request |
| `archive_selection_of_100001_cancelled_early`, Cancel at 200 files | job ended 114 s and 94 s after the request (two runs); the first file came 33 s and 17 s after the enqueue |

The 100,001-member run reproduces the acceptance's 114–117 s with a UI that takes every
event at once, so the notices were not the cause. A traced run (150 s, machine fully
loaded) put 148.8 s of it in the job's final namespace revalidation
(`Claim::revalidate_before_close`). The rest was small: `finish_in_place` 10 ms,
dropping the job 145 ms, releasing the retentions 1.5 s. The copy itself wrote no file
after Cancel.

A later timed run split that revalidation. It took 131.5 s, and 127.7 s of it went to
collecting the pins: each root was tested against a list of the excluded roots. The
native refreshes of the 100,001 pins took only 3.5 s.

The fix, approved by the Architect:
- The close now probes only the pins of the items the job started, the roots its turns
  touched and its private pins.
- Refreshes test excluded roots against one bit per planned root.
- `a_cancelled_selection_closes_with_only_the_members_it_took` counts the refreshes.

After the fix, Cancel at 200 files ended the job in 1,013 ms and 1,433 ms (debug), and
in 496–523 ms in release (release profile without fat LTO, 16 codegen units).

Admission still probes every root's pin, so the first file comes 17–42 s after the
enqueue in debug and 21–22 s in release. Its breakdown (about 400 µs per member) and a
proposal are COPY-06 in the [backlog](../implementation-backlog.md).

### Release run (release profile without fat LTO, 16 codegen units)

Before is the export's first harness commit (`f2e8db7d`: base, probes, measurement
tests, none of the fixes), with the 100,001-member test added. After is the final
export. Runs are listed in order, on a machine kept busy by other builds.

| Run | Folder files/s | Selection files/s | Cancel of 100,001 at 200 files | First file |
|---|---|---|---|---|
| after, 1 | 201 | 190 | 495 ms | 19.6 s |
| before, 1 | 101 | 80 | — | — |
| after, 2 | 167 | 165 | — | — |
| before, 2 | 105 | 125 | 9,650 ms | 18.7 s |
| after, 3 | — | — | 402 ms | 22.9 s |

### Cancel, pause and the UI

- A UI that took no events while a selection of 6,000 members was copied
  (`archive_selection_cancel_while_the_ui_takes_no_events`): Cancel at 4,335 files wrote
  no further file. The terminal progress arrived after 273 events and 1,111 ms with
  joined notices, and after 4,362 events and 1,214 ms with one notice per root (the
  former delivery, switched back on for this run).
- Pause and Cancel during a batched copy (`archive_copy_pause_and_cancel_latency`,
  3,000 members): the paused progress came 0.3 ms after the request; no file was written
  in the 500 ms after it; Cancel ended the job 287 ms after the request. A batch ends at
  the next step boundary, so the 5 ms limit does not delay either.
- Presenter (`completing_a_large_archive_selection`, 100,001 members):
  - 100,001 completions took 169 ms as single-root notices and 75 ms as notices of 512.
  - Deselecting them took 25 passes and 96 ms in all.
  - With the folder-size observer keeping each selection it was sent, it took 49 passes
    and 24 backend copies of the set.
- One UI refresh of a panel of 100,001 selected archive members took 0.5 ms, and one draw
  9.8 ms (`refreshing_a_panel_of_100001_selected_archive_members`).
- Engine deselection of 100,001 copied members in edits of 64 (`removal_cost`):
  0.08 ms per edit, against 27.37 ms per edit and 43 s in all with the former copy and
  recount.

## Tests, red then green

The red runs used temporary switches that restored the former behaviour, in the same
build; the switches were removed before the final build.

| Test | Red | Green |
|---|---|---|
| `waiting_root_notices_reach_the_ui_joined_and_before_the_terminal_progress`, `a_cancelled_selection_reaches_a_waiting_ui_after_a_few_events` | one notice per root: 0 passed, 2 failed | 2 passed |
| `an_aborted_actor_ends_a_batch_after_the_step_in_flight`, `cancel_and_pause_end_a_batch_at_the_next_step_boundary`, control `a_batch_of_steps_runs_a_small_member_to_its_completed_root` | batches ignoring abort, Cancel and pause: 1 passed (the control), 2 failed | 3 passed |
| `removing_a_large_selection_in_rounds_does_linear_work_with_exact_totals` | copy and recount per edit: 234,478,134 entries visited for 100,001 removals, at most 200,002 allowed | passed |
| `copied_archive_members_leave_the_panels_own_selection_in_place`, `a_large_shared_archive_selection_is_copied_once_then_changed_in_place`, and the adapted `archive_completion_uses_mount_identity_and_waits_for_pending_selection` | every removal through the backend: 3 failed | passed |
| `a_cancelled_selection_writes_no_member_after_cancel` | a guard: the copy itself already stopped at once before this pass | passed |
| staging folder and stage descriptors (`transfer::destination::staging_tests`, 10 tests, and 4 engine tests) | per-file stages: 9 of 10 and 4 of 4 failed; descriptor-less stages: the 3 descriptor tests failed; a leaking guard: its test failed | passed |
| handle proof (`transfer::namespace::parent::proof_tests`, 12 tests) | with parts of the proof left out: up to 10 of 12 failed | 12 passed |
| close of touched pins only (`a_cancelled_selection_closes_with_only_the_members_it_took` and 8 more, 2 of them bitset tests) | close probing every pin, exclusions not tested: 8 of 11 failed; the 100,001 case refreshed 100,001 pins in 149.8 s | 11 passed; the 100,001 case refreshed 11 pins in 2.2 ms |
| `a_copy_skips_members_that_need_a_password_that_was_not_entered` | the member asked about like any other: failed, "the copy asked about a member: archive password: …" | passed |

Final runs, on the final debug build:
- engine lib: `copy archive selection delivery drag_drop tree_move namespace` gave 802
  passed; the whole library gave 1,925 passed and 1 failed. The failure was
  `watch::tests::a_panicking_probe_restarts_the_worker_at_once_then_at_most_once_per_interval`,
  a watch timeout under load in code this pass did not touch; alone it passed 3 of 3.
- UI presenter: 684 passed.
- platform-windows `transfer::`: 163 passed.
- filesystem host: 115 passed.
- core: 134 passed.
- platform-common: 167 passed.
- engine integration `archive_selection`: 5 passed.
- UI integration `archive copy`: 106 passed.
- `cargo clippy … --lib --tests -- -D warnings` on the six crates: clean.
- `cargo fmt --check`: clean.

The host's completion tests used the second and third archive validation of an exchange
as the moment to stop or cancel it. These now use the same moments through the source's
cleanup-status checkpoint, which the session still reads there.
