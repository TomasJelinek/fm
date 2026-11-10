# Real operation and advanced Find qualification

This is a working qualification record for the [accepted scope](../superpowers/specs/2026-11-10-operations-find-qualification.md).
It does not mark the broader application goal complete or establish a new release.
Logs and retained owned fixtures are under `_build/qa/operations-find`.

## Verified native operations

`cargo test -p midnight-engine --test operations_real_windows --locked --offline
--target-dir _build -- --nocapture --test-threads=1` ran as part of
`native-green.log`: **14 passed**, one explicitly opt-in recycle test omitted,
4.81 seconds for the target. The production Engine/native services operated on
new owned Windows fixtures, rather than shell copy/delete commands.

The corpus contains 271 files and 9,996,785 bytes, Unicode and long names,
nested/empty folders, empty/small files and a larger streamed file. Full SHA-256
copy and size-plus-three-samples copy both passed independent final SHA-256
comparison. Whole-directory same-volume Move preserved native identity with
zero content reads/copy/checksum bytes. Directory merge and progressive source
removal were checked separately.

Coverage includes overwrite Skip/Rename/Cancel/Replace, a late publication
collision, self/hard-link/descendant rejection, nine queued jobs, queued
cancellation, actual missing-directory retry, stale decision rejection, actual
Windows sharing-denial retry, source pinning against concurrent writes and
namespace changes, partial recursive permanent deletion, read-only hard-link
sentinels, and cancellation before publication at Copying/Syncing/Verifying/
ReadyToPublish for both verification policies. Deterministic phase gates control
timing around real native I/O; these are not physical device-removal tests.

The opt-in `real_recycle_owned_fixture_without_permanent_fallback` test passed
**1/1**, 0.09 seconds, in the normal Windows desktop context. It recycled only
one newly created owned folder and did not enumerate or empty the Recycle Bin.
Its first sandbox run failed with `COPYENGINE_E_ACCESS_DENIED_SRC` (`0x80270021`)
and preserved the source; the unchanged test succeeded outside that restriction.
Logs: `recycle-native.log` and `recycle-desktop.log`. This is an environment
distinction, not a production workaround or permission relaxation.

Detailed fixture expectations: [real operation QA](../developer/OPERATIONS_REAL_QA.md).

## Verified Find backend

`backend-green.log` records 5 advanced-query tests, 7 duplicate failure tests,
and 5 sort tests passing. Relative age uses one frozen acceptance timestamp;
old parent directories remain traversable. Duplicate groups prune unique sizes,
stream SHA-256, compare exact bytes and validate retained identities/revisions.
Groups stay adjacent under sorting and are published atomically.

`native-green.log` additionally records **8 streaming content tests**, **2
Unicode tests** and **1 actual Windows read-sharing failure test** passing.
The latter holds a real conflicting Windows handle: content/duplicate searches
report incompleteness and the affected path, then a fresh search succeeds after
the handle is released. Injected mutation/read failures in other targets remain
explicitly distinguished from this native failure.

`unicode-after.log` subsequently records **5/5 tests**, including the opt-in
throughput probe, malformed-file recovery across reused matchers, mixed UTF-8/
UTF-16 encodings, fold expansions, whole-word boundaries and cache collisions.
The fold cache is bounded to 512 checked scalar entries and allocated only when
needed; reads remain bounded to 64 KiB and files are never loaded wholesale.

## Measured performance

These are single local debug runs, not release throughput guarantees:

| Probe | Before | After | Scope |
| --- | --- | --- | --- |
| 96 MiB ASCII, case-insensitive | 3,250 ms | 1,124 ms | Generated streaming source; query/buffer reuse and ASCII folding |
| 96 MiB ASCII, case-sensitive | 658 ms | 698 ms | No demonstrated improvement; ordinary measurement variation |
| 8 MiB multilingual whole words, no match | 1,095.597 ms | 407.868 ms | Repeated scalar folds cached |
| 8 MiB multilingual whole words, late match | 1,080.026 ms | 406.678 ms | Same input/query before and after |
| Duplicate candidate pruning | — | 147.461 ms | 2,004 native entries; 40,960 bytes read after size pruning |

Logs: `content-baseline.log`, `content-after.log`, `unicode-baseline.log`,
`unicode-after.log`, `duplicates-performance.log`. Generated-source matching
does not measure physical disk throughput. Corpus-copy elapsed times are
correctness-fixture timings, not a comparison of verification-policy speed.

## Still being qualified

- Installed UI-to-engine keyboard, toolbar, clipboard and internal folder-drop
  pathways; the software-window harness is not desktop OLE interoperability.
- Find interaction/reflow/rendering and full UI-to-native search composition.
- Final strict lint, independent review, debug build/deployment and native smoke.
- Linux runtime qualification of these new features remains a separate gate.

The initial UI qualification build caught one outdated `FindRow` test fixture
after adding duplicate-group fields; the fixture was corrected. That failed
compile is retained in `find-ui-green.log` and is not claimed as a passing run.
