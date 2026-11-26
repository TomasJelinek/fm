# Real Windows operation QA

The integration target `operations_real_windows` exercises the production Engine,
LocalFileSystem, and VerifiedFileCopy services on real files. It creates fresh,
uniquely named directories beneath `_build/qa/operations-find/transfer-real-*`.
A passing case removes its directory. A failing case keeps it with its per-case
`EVIDENCE.txt` for inspection, and `MIDNIGHT_KEEP_TEST_FIXTURES=1` keeps the directories
of passing cases too ([test fixtures](TEST_EXECUTION.md#test-fixtures-under-_buildqa)).
The presence of a directory is not a pass.

## Expected results

A conflict Skip ends the job `Completed` with the skip count, and only a skipped I/O
error ends it `CompletedWithErrors`; invalid source ancestry fails at once without a
Retry prompt; a later queued cancellation acknowledges `CancelRequested` before its FIFO
terminal event; and Windows staging deletion can stay pending until the owned native
handle closes, which the engine does before terminal progress. The target tests the
backend and native APIs; it does not automate a physical keyboard, mouse or Shell
handoff. The run time of `real_directory_corpus_matches_independent_sha256_for_both_policies`
is tracked as PERF-04 in the [backlog](../IMPLEMENTATION_BACKLOG.md). Recorded run:
[operations and Find record](../verification/2026-11-10-operations-find.md).

## Running the checks

Run from the repository root using the normal repository Rust environment:

```powershell
cargo test -p midnight-engine --test integration operations_real_windows:: -- --nocapture --test-threads=1
```

The manual recycle case uses the production dedicated Windows recycle service on
one freshly generated owned folder. It does not enumerate or empty the Recycle
Bin. Run only when deliberately exercising that integration:

```powershell
cargo test -p midnight-engine --test integration operations_real_windows::real_recycle_owned_fixture_without_permanent_fallback -- --ignored --nocapture --test-threads=1
```

## Coverage and evidence

| Case | Real boundary and assertions |
| --- | --- |
| Corpus Copy, default SHA-256 and explicit samples | 271 files, empty directory/file, nested Unicode, decomposed names, literal shell characters, long paths, read-only source, 257 small files, 64/192/256 KiB boundaries, and an 8 MiB file. Independent full SHA-256 manifests verify both policies' outputs and retained source. |
| Directory Move | Actual same-volume native rename; directory identity preserved; copied, verified, and validation byte counts remain zero; every descendant matches the prior manifest. |
| Overwrite choices | Skip, Rename, Cancel, and recoverable replacement. The current source remains intact; displaced original bytes remain at the reported recovery path. |
| Native Move replacement | Actual source namespace removal, zero copied bytes, and retained displaced destination. |
| Destination Retry | Real missing destination fails twice, then succeeds after creation; new conflict has a fresh decision version; stale replacement decision cannot replace the new winner. |
| Locked source Retry | Real Windows exclusive sharing handle blocks preparation; Retry still fails while held and succeeds after explicit handle release. |
| Queue cancellation | A first job's real overwrite conflict gates nine jobs. A queued job cancels without destination writes; remaining jobs complete. |
| Identity/ancestry | Self-copy and hardlink aliases cannot authorize replacement; a forged replace choice is refused; copying into a source descendant fails before writes. |
| Directory Move merge | Explicit merge keeps existing destination children, transfers source descendants, and removes the finished source directory. |
| Cancellation gates | Actual native file writes, sync, and readback precede exact state-machine gates for Copying, Syncing, Verifying, and ReadyToPublish under both policies. Explicit cleanup succeeds and native handle release removes the unpublished stage, matching the engine's pre-terminal cleanup boundary. |
| Late publication collision | A real competing target appears after verification. Publication fails without replacing it; Rename publishes the retained verified stage without copying again. |
| Source protection | Actual write, rename, and delete attempts against a pinned source fail under Windows sharing rules. |
| Permanent deletion | Recursive real corpus deletion cancels after exactly one callback, checks every remaining entry against the initial manifest, then completes. A read-only hardlink outside the selected tree retains bytes and read-only state. |
| Delete identity | Pre-cancelled permanent/recycle requests preserve source bytes; a prepared deletion refuses a real replacement at the original path. |
| Recycle, manual only | Dedicated native recycle accepts a fresh owned folder, or reports Unsupported while preserving it. An unrelated owned sentinel remains. No permanent fallback is used. |
| Recycle refused, manual only | With `MIDNIGHT_TEST_NUKE_DIR` naming an owned folder on a drive whose Recycle Bin removes files immediately (`NukeOnDelete`), the Shell refuses at once with `COPYENGINE_E_RECYCLE_FORCE_NUKE` (0x80270036); the refusal is trash-unavailable (the job then asks whether to delete permanently) and the file stays: `cargo test -p midnight-platform-windows --lib trash_on_a_volume -- --ignored --nocapture`. |

The full engine tests execute the same backend queue used by F5/F6, clipboard
paste, and confirmed native drag/drop. UI input routing needs its own presenter
tests; passing this integration target does not claim a physical keyboard, mouse,
or Shell drag/drop session was automated.

## Fault injection and environmental limits

The phase gates above are deterministic boundaries between real native calls.
They do not claim to interrupt an in-flight OS write at an arbitrary byte.
Engine unit tests supplement those boundaries with controlled native adapter gates
and injected errors:

```powershell
cargo test -p midnight-engine --lib queued_cancel_removes_only_unpublished_output_after_blocked_native_io -- --nocapture
cargo test -p midnight-engine --lib partial_disk_full_write_pauses_and_retries_without_corrupting_or_recopying -- --nocapture
cargo test -p midnight-engine --lib cancel_partial_write_cleans_stage_and_releases_the_next_queued_job -- --nocapture
```

The first case gates real native adapter I/O and forces the verified cross-device
Move route. The latter cases inject disk-full/partial-write failures; they do not
fill a real volume. No disk exhaustion, physical device removal, network failure,
or actual cross-volume move is claimed by this target. Cross-volume testing needs
separately owned fixtures on two approved volumes. Windows Trash availability and
its own path limits remain environmental constraints. The manual test prints
`UNSUPPORTED` explicitly when the platform cannot recycle an item.

Sample verification is intentionally weaker than full SHA-256: it reads first,
middle, and last 64 KiB ranges, comparing files up to 192 KiB completely. The QA
manifest always hashes complete output bytes independently; that confirms the
observed test result without upgrading the product's sampled guarantee.

## Salamander comparison used for the matrix

The scenario categories follow Salamander's copy/move worker: separate Skip All state
for read, write, overwrite, move, delete, directory creation and metadata errors, and
explicit Retry and Cancel. The suite does not claim feature equivalence for Salamander's
alternate streams, encryption, network reconnect, archive or plugin operations.
