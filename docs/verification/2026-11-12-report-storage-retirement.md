# Diagnostic report native ownership: investigation and next contract

Status: both native ownership defects reproduced on Linux; no production fix in this batch.
The temporary regression patch and original output are in `_build/qa/report-ownership-20261112`.
The existing startup ledger does not yet include a report arena.

## Actual owners and escape paths

`size_warnings::State.storage` owns a boxed native `AppendSpool`. A published
`FolderSize.warnings` retains `FrozenReport -> ReportMark -> Arc<Mutex<State>>`. A final result,
cache entry or warning-handle clone can therefore release the file outside its producer worker.

`size_warnings::Text.body` owns an independent `SpoolPrefix`. Tree and presence diagnostics retain
that text through `TreeDiagnostic.details`; composed tree reports retain the same sources.
`AppendSpool` and every prefix share the same `Arc<Mutex<File>>` and private-directory root pins.
Dropping the writer does not close the file while a prefix remains, but the final prefix can close
it on its caller. Original-service `TaskLifetime` anchors do not own these newly created files.

Current workers already protect their rejected blocking sends. The UI also has a retirement
queue. Those protections do not make the concrete file owner intrinsically safe when a delivered
result, unconsumed output, queued request or independent clone becomes the last owner elsewhere.
Preserve those existing boundaries while fixing report ownership at its creation point.

There is no valid fixed bound of four reports: four root workers reuse logs only within one scan;
entry-size maps retain results from multiple scans, and `tree_sizes::partial` can create a new log
for each failed row. A guessed per-report semaphore could exhaust while the same producer retains
all existing reports, preventing progress. No warning details may be silently discarded to fit it.

## Reproduction contract

The temporary patch adds two focused tests to `size_warnings` without changing production behavior.
Construction, writes and full-content validation run on a native worker. The first publishes a real
`FolderSize`, clones it, and observes the actual boxed `AppendSpool` destructor when the last result
is dropped on the caller. A destructor gate keeps the real spool alive if cleanup moves off caller;
it deliberately does not block the broken caller-side path.

The Linux test publishes an actual `TreeDiagnostic` using `tree_diagnostics::report`. It identifies
the private anonymous regular-file descriptor by its unique fixture directory and device/inode,
drops the writer, verifies the prefix still owns that file, then drops the last diagnostic clone.
A correct guarded owner must retain that descriptor while native cleanup is gated. This tests the
actual `SpoolPrefix` close, not just a fake service or metadata destructor. Reports include long
Unicode text and a unique tail, and reads verify the complete contents before ownership assertions.

The driver restores the original module bytes in `finally`, checks its SHA-256, and removes only
unchanged temporary test copies. Failing tests are retained as a QA patch, not registered in the
production test graph until the corresponding fix is ready.

## Recorded RED and exact restoration

Native Linux engine compilation completed in **1m39s**. Both selected tests reached their intended
ownership assertions and failed in **0.08s** (zero passed, two failed, 967 filtered). The original
`linux-red.log` records:

- `report_retirement_folder_size_final_owner_destroys_actual_storage_off_caller`: actual
  `AppendSpool` owner destructor thread equals the caller thread (`ThreadId(4)`).
- `report_retirement_tree_diagnostic_prefix_keeps_file_until_worker_cleanup`: the exact anonymous
  file descriptor is gone immediately after the final `TreeDiagnostic` prefix is dropped.

Both tests first validated complete long Unicode report contents on their native construction
worker. The prefix test also verified that dropping the writer and a non-final diagnostic clone
kept the actual file alive. Failures therefore identify final native ownership, not failed creation,
truncation, a guessed handle or an unrelated metadata destructor.

Command: native WSL Ubuntu 24.04, existing `linux-verification-env.sh`, locked/offline SDK config
`_build/qa/elevation-linux-20261111/sdk.toml`, `CARGO_BUILD_JOBS=2`, isolated native `TMPDIR`, then
`cargo test -p midnight-engine --lib report_retirement -- --test-threads=1 --nocapture`.
The exact executable command, fixture root and temporary patch are preserved alongside the log.

The driver's `finally` restored `size_warnings.rs` byte-for-byte and removed both temporary test
files. Restored source SHA-256:
`91f1ca263fde6e73d104ee97ec9952ab6c39f653a60cdecb2acf0f14e366ebfd`.
A subsequent Git diff of that source was empty, and both temporary source paths were absent.
The QA artifact `evidence.sha256` records the log, patch and baseline hashes. The Linux target was
then handed to the archive owner for a fresh final build; the temporary executable is not a final
qualification artifact. No arena code or failing test registration remains in production source.

## Smallest proposed arena

This is a design for review, not implemented behavior:

- Reserve one startup retirement lease for an `Arc<WorkerOwned<SharedArena>>`. Start one fallible
  standard cleanup worker before accepting native report ownership. No thread is created by Drop.
- Keep independent append files. Before opening each file, reserve a reusable arena node and its
  final-owner token. A node records the accepted native storage and intrusive pending/free links.
  Allocation failure happens before file creation and remains a reported storage failure.
- The registry retains a strong native storage owner. Writer handles contain their raw storage
  reference followed by the shared token; text bodies contain `SpoolPrefix` followed by that token.
  Field order must release every raw prefix/storage reference before the final token is destroyed.
- Final token Drop only queues its already reserved node index under a short metadata mutex and
  signals the worker. It allocates no node, performs no filesystem work, waits for no cleanup, joins
  no thread and requires no Tokio runtime. Clones share the token rather than allocating new nodes.
- The cleanup worker extracts one queued storage owner, releases the registry mutex, then performs
  its actual destruction. Only after that destruction may the node be recycled. Files whose tokens
  remain live are never removed by unrelated report cleanup or engine shutdown.
- The queue has one possible entry per accepted native file; it is not an unbounded secondary queue
  of completed payloads. Dynamic nodes follow the existing accepted-file population, with reusable
  metadata capacity at its high-water mark. This does not impose a new arbitrary report-count cap.
- A metadata-only control owner is shared by the engine and tokens. The worker holds the guarded
  registry but not that control owner, avoiding a cycle. The final control owner closes intake and
  wakes the worker; normal shutdown drains queued resources without joining on the caller.

Cleanup must catch individual destructor unwinds outside the registry mutex and recycle a node
only after its payload has actually unwound. Unexpected worker exit must mark the arena unavailable
before further file creation. The guarded registry remains a last-owner safety net for existing
files and tokens if incremental cleanup stops. Such a failure can delay reclamation until remaining
owners disappear; it must not silently resume accepting files or be described as normal timely
cleanup. This exceptional path needs an explicit gated regression before implementation is accepted.

This preserves per-file failure isolation, immutable range semantics, existing bounded read size,
complete report contents and normal final-handle reclamation. A shared indexed session append file
was rejected: it would retain completed data for the session, change read I/O and failure scope, and
require substantially more than an ownership correction.

## Integration decisions and acceptance gates

The proposed seam is `ReportArena::try_start(startup_lease) -> io::Result<ReportArena>` before native
services are accepted, with an explicit cloned arena/factory passed into `ReportLog` construction.
Root owns engine startup integration. Do not hide fallible worker creation in report Drop or use
thread-local context that can be absent on detached native threads.

Explicit factory plumbing must cover folder-size lanes, entry/selected/tree sizes, archive-size
scans, folder presence, the provider directory-tree model, tree refresh/watch failures and standalone
`tree_diagnostics::diagnostic` calls. `DirectoryTree::new` is currently public within the provider
surface; any signature change needs coordination rather than an incidental public API rewrite.
No arena should wrap pure metadata or retain an original service merely to keep report files alive.

Before production integration: make both native RED cases pass; cover writer/prefix clone orders,
completed-but-unconsumed outputs, runtime shutdown, cancellation before work starts, startup failure,
file creation failure, worker/destructor panic, node reuse and a gated full cleanup backlog. Re-run
existing storage-full/partial-record, frozen-range, Unicode, complete-report, cancellation and private
spool security tests. Check all producer routes use the explicit factory, and retain existing UI
retirement and native rejected-send protection. The current batch carries evidence and this contract;
implementation follows as its own ownership batch.
