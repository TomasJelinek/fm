# Engine, core and work budget review — 2026-11-24

## Scope

The owner's goal: review the code for correctness, threading safety, memory ownership,
data structures and speed; measure the tests and make them faster without losing
coverage; remove obsolete code and tests. This pass covered `midnight-engine`
(about 3.0 MB of non-test source), `midnight-core` and `midnight-work-budget` at
`ac525b6f`, within a time box of about three hours. It is a partial review: what was read
is listed below, and nothing beyond it is claimed.

## Coverage

Read fully:

- `midnight-work-budget`: `lib.rs` (FIFO I/O gate, retained permits) and `resident.rs`
  (resident worker pool, shutdown).
- `midnight-engine`: `copy_operations.rs` (copy actor, admission, pause, leases, progress
  cadence), `folder_sizes.rs` and `folder_sizes/batched.rs`, `find_worker.rs`,
  `find_duplicates.rs`, `find_content.rs`, `find_text_matcher.rs`, `comparison/walk.rs`,
  `comparison/contents.rs`, `user_command_process_tracker.rs`, `checksum_format.rs`.
- `midnight-core`: `provider.rs` (`Cancellation`), `job_pause.rs`, the `CompareMarks`
  part of `compare.rs`, `DirectoryIndex` in `entry.rs`.

Read in part: `copy_worker.rs` (job state, `steps`, `store_progress`),
`copy_namespace.rs` (registry state, `begin`, `reserve_private_child`, `finish`),
`copy_input.rs` (`resolve`), `checksum_manifest.rs` (`encode`), `file_targets.rs`
(`resolve_within`), `watch.rs` and `watch/signature.rs` (per-entry child check).

Not reviewed: the rest of `midnight-engine` (tree controller and actor, watch supervisor,
operations and metadata workers, archives, drag and drop, selection, desktop, providers,
user command preparation and output, the other copy modules) and the rest of
`midnight-core`. There is no `unsafe` code in the three crates
(`#![forbid(unsafe_code)]` in both libraries; none in `midnight-work-budget`).

## Findings

| Severity | Area | Finding | State |
| --- | --- | --- | --- |
| Medium | Folder sizes | Sizing a network share root counted none of its top-level entries: Windows resolves the root to `\\?\UNC\server\share` without a final separator, and `is_immediate_child` rejected every child it listed (`\\?\UNC\server\share\name`) as "a path outside the immediate directory". Reproduced with `\\localhost\c$`. | Fixed |
| Medium (speed) | Folder sizes | The same per-entry check parsed both paths for every enumerated entry: 320 ns per entry (release, 1,000,000 children), comparable to the enumeration itself. | Fixed: 33 ns |
| Low–medium | Compare directories | With Include subfolders, a child that one side could not describe (`FindChild::Failed`) was dropped before pairing, so its counterpart was marked *only here* and everything inside it inferred *only here*; the `compare_group` arm meant to mark such pairs not verified was unreachable. | Fixed |
| Low | Job pause (core) | A resume between a checkpoint's read of the request and its acknowledgement left the pause acknowledged with no request; the next pause reported `is_paused()` at once, before any checkpoint reached it (a copy shows Paused while a native step still runs with its disk slot). | Fixed |
| Test time | Engine integration executable | `checksums_backend` and `checksum_manifest_verification` each compiled `src/checksum.rs`, `src/retirement.rs` and `src/verified_output.rs`, so 94 library unit tests ran twice there (and once more in the library), among them the any-size manifest tests of 4.3–4.6 s each. | Fixed |
| Low (speed) | Watch | `watch.rs` (`probe`) and `watch/signature.rs` checked every listed child with the same `strip_prefix` and component parse (about 0.3 s of CPU per probe of a 1,000,000-entry folder), and would reject the children of a verbatim share root the same way. | Fixed: they use the folder-size check |
| Follow-up (test time) | Engine library | `copy_operations::input::root_bound_tests::a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared` (9.5 s) is the library executable's critical path; most of it builds 1,000,001 entries and their index. Also, `resolve` collects every selected path before refusing more than 1,000,000; refusing from the selection's count first would avoid that work in production. | Open |

Reviewed without a finding: the I/O gate's FIFO admission, wake chain, withdrawal on drop
and `held`/counter consistency; the resident pool's claim/abort ordering and shutdown;
the copy actor's lock order (registry, then status, then pause requests; status before
throughput tracker everywhere) and its `Controls` wait (the notification is enabled before
the flag check); folder-size frontier and ancestry bounds; Find's retained-byte budget,
identity cycle check and duplicate grouping; the content reader pool's queue bounds and
shutdown; the process tracker's slot reservation and reaping. Design choices noted, not
changed: Find reads a matching file to its end so that malformed UTF-8 after a match is
still reported (ACCEPTANCE `find_query_syntax`); duplicate byte comparison re-reads the
bucket representative for each member.

## Fixes and evidence

Each fix has a test that failed before it (Windows, `cargo test --locked --offline`):

1. **Folder sizes, share roots and speed** (`folder_sizes.rs`). New tests
   `folder_sizes::tests::only_a_single_normal_name_below_the_directory_is_an_immediate_child`
   and (Windows) `the_children_of_a_verbatim_share_or_drive_root_are_immediate_children`;
   the second failed before the fix (`tests.rs:906`, `\\?\UNC\server\share`). The check
   now accepts the `directory.join(name)` form from its bytes (absolute directory, one
   main separator, a name without separators that is not `.` or `..`) and otherwise
   compares components, allowing the root component that a verbatim prefix-only
   directory lacks. Measured with a standalone release build (`rustc -C opt-level=3`),
   1,000,000 children, three rounds each:

   | Directory | Before | After |
   | --- | ---: | ---: |
   | a local folder (`C:\…\deps`) | 320–341 ns per entry | 30–36 ns per entry |
   | the same folder, verbatim (`\\?\C:\…`) | 255–264 ns | 38–40 ns |
   | `\\?\UNC\server\share` | 99–119 ns, **0 accepted** | 45–57 ns, all accepted |

   `watch.rs` (`probe`) and `watch/signature.rs` now use the same check (the signature
   also still requires the entry's own name to be the path's last component); all
   `watch` library tests pass.
2. **Compare directories, unreadable children** (`comparison/walk.rs`). New test
   `comparison::deep_tests::an_unreadable_child_leaves_its_counterpart_unverified_not_one_sided`
   failed before the fix (`Some(OnlyHere)` for the counterpart). The listing keeps the
   names of the children it could not describe (charged to its budget) and the walk marks
   their counterparts *not verified*; the folder itself stays not verified as before. All
   42 `comparison::` tests pass.
3. **Job pause** (`midnight-core/src/job_pause.rs`). New test
   `provider::pause::tests::a_pause_after_a_resume_that_raced_a_checkpoint_waits_for_the_next_checkpoint`
   steers the interleaving through a test-only hook between the checkpoint's read and its
   acknowledgement; it failed before the fix (`job_pause.rs:117`). A pause that was not
   requested before now starts unacknowledged; repeating the request of an acknowledged
   pause keeps it acknowledged (the copy actor re-applies its requests on every
   confirmation step). The 26 `pause` tests of `midnight-core` and `midnight-engine` pass.
4. **Duplicate test runs** (`tests/checksums_backend.rs`,
   `tests/checksums_backend/manifest_verification.rs`). The manifest verification tests
   became a submodule of `checksums_backend` and use its includes; their five tests are
   unchanged. The integration executable lists 94 tests fewer (all
   `checksum_manifest_verification::{checksum,retirement,verified_output}::…`, each still
   in the library and once in `checksums_backend`). Two runs each, same machine, nothing
   else building:

   | Integration executable | Summed test time | Wall time |
   | --- | ---: | ---: |
   | Before | 63.0 s, 68.3 s | 5.29 s, 5.28 s |
   | After | 46.7 s, 48.7 s | 4.76 s (second run; the first, 10.9 s, ran cold) |

   The wall time stays bounded by the one remaining copy of the any-size manifest tests
   (4.6 s): no `cfg` tells the library's test build from an `#[path]` include (see the
   [test speed pass](2026-11-21-test-speed-pass.md)).

## Test times measured

Windows, debug test profile, `--format json --report-time` (`RUSTC_BOOTSTRAP=1`), before
the changes: `midnight-core` library 169 tests in 0.11 s and its integration executable 165
in 0.51 s; `midnight-work-budget` 50 tests in 0.03 s; `midnight-engine` library 1,993
passed, 30 ignored, slowest 9.5 s (above), then 6.3 s
(`copy_operations::tests::streamed::a_streamed_move_across_volumes_…`), 5.5 s
(`copy_operations::tests::archive::a_cancelled_selection_reaches_a_waiting_ui_…`) and the
any-size manifest tests (5.3 s, 4.9 s); `midnight-engine` integration executable 506
passed, 13 ignored, wall 5.3 s.

## Validation

After all fixes, Windows: `cargo test -p midnight-core -p midnight-engine
-p midnight-work-budget --locked --offline` passed (core 170 and 165 with 4 ignored,
engine library 1,995 with 30 ignored, engine integration 413 with 12 ignored, work budget
50); `cargo clippy` of the three crates with `--all-targets -- -D warnings` and, for the
libraries and tests, with `--target x86_64-unknown-linux-gnu` was clean; `cargo fmt
--check` was clean.

## Not done

The workspace-wide clippy with the acceptance feature, Linux test runs and acceptance
were not run: no other crate changed and no acceptance scenario covers these paths
beyond the unit tests above.
