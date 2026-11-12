# Real Windows operation QA

The integration target `operations_real_windows` exercises the production Engine,
LocalFileSystem, and VerifiedFileCopy services on real files. It creates fresh,
uniquely named directories beneath `_build/qa/operations-find/transfer-real-*`.
Fixtures and per-case `EVIDENCE.txt` files remain available for inspection. A test
failure leaves its evidence on disk too; the presence of a directory is not a pass.

## Recorded execution, 2026-11-10

The ordinary target completed with **14 passed, 0 failed, 1 manual recycle case
ignored**, in 4.81 seconds. The corpus held 271 files totaling 9,996,785 bytes.
Full verification read all 9,996,785 bytes; sampled verification read 1,673,696
bytes. Both copied trees matched independent full-file SHA-256 manifests. The
same-volume directory Move retained native identity and reported zero content
copy, verification, and validation bytes. Eight exact cancellation gates passed
across the two verification modes, and all retained-stage directories were empty
after explicit cleanup and native handle release. Evidence is in
`_build/qa/operations-find/native-green.log` and the individual retained fixtures.

The explicit recycle test first failed inside the restricted execution sandbox
with `0x80270021`, the pinned Windows SDK's `COPYENGINE_E_ACCESS_DENIED_SRC`.
Running the **same built test** outside the sandbox against a fresh owned fixture
then passed, 1/1 in 0.09 seconds. The sandbox failure and successful desktop-context
run are retained separately in `recycle-native.log` and `recycle-desktop.log` under
the same QA output directory. No permanent deletion fallback or global Recycle
Bin cleanup occurred.

Initial harness runs exposed incorrect QA assumptions, subsequently corrected:
Skip produces `CompletedWithErrors` with a skip count; invalid source ancestry
fails immediately without a Retry prompt; a later queued cancellation acknowledges
`CancelRequested` before its FIFO terminal event; and Windows staging deletion
can remain pending until the owned native handle closes. The engine performs that
handle release before terminal progress. No production transfer fix was justified
by those observations. Earlier failed/incomplete logs remain as diagnostic history.

These are backend and native API results. They do not qualify a full release or
claim physical desktop keyboard/mouse/Shell handoff automation.

## Running the checks

Run from the repository root using the normal repository Rust environment:

```powershell
cargo test -p midnight-engine --test operations_real_windows -- --nocapture --test-threads=1
```

The manual recycle case uses the production dedicated Windows recycle service on
one freshly generated owned folder. It does not enumerate or empty the Recycle
Bin. Run only when deliberately exercising that integration:

```powershell
cargo test -p midnight-engine --test operations_real_windows real_recycle_owned_fixture_without_permanent_fallback -- --ignored --nocapture --test-threads=1
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

The full engine tests execute the same backend queue used by F5/F6, clipboard
paste, and confirmed native drag/drop. UI input routing needs its own presenter
tests; passing this integration target does not claim a physical keyboard, mouse,
or Shell drag/drop session was automated.

## Fault injection and environmental limits

The phase gates above are deterministic boundaries between real native calls.
They do not claim to interrupt an in-flight OS write at an arbitrary byte.
Existing engine unit cases supplement those boundaries with controlled native
adapter gates and injected errors:

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

The local reference tree `D:\experiments\salamander-main\src` was inspected without
changes. Its `worker.cpp` defines separate Skip All state for read, write,
overwrite, move, delete, directory creation, and metadata errors (around lines
989–1012), and explicit Retry/Cancel dispatches (including around lines 2517–2592).
Its copy/move dialog and drag/drop call paths live in `fileswn8.cpp`,
`fileswn6.cpp`, and `fileswnb.cpp`. These paths informed the scenario categories;
this suite does not claim feature equivalence for Salamander's alternate streams,
encryption, network reconnect, archive, or plugin operations.
