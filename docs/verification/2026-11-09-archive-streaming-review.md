# Independent archive streaming review and completion regression

Date: 9 November 2026. Reviewed against HEAD `eb86033` and the original
`_build/archive-streaming-implementation/source-hashes.json` freeze. This review
is independent of the implementation review. It excludes subsequent PFS02 archive
folder-size work, unrelated native/network filesystem ABI work, UI changes and
custom archive suffix policy.

## Result and corrected finding

One actionable P2 defect was found and corrected. No further actionable defect
was found in the reviewed streaming, staging, native spool or resource-policy
paths. This is a scoped source/evidence review, not a proof against all malformed
archives or operating-system failures.

**P2: a terminal result could escape after the supervisor stopped the mount.**
In `crates/midnight-filesystem-host/src/session.rs`, cached and newly staged reads
performed blocking source validation after the last watchdog-state check. Cached
reads checked cancellation but not `supervision.stop`; generic terminal replies
also lacked a final stop/cancel check. The real installed-worker probe delayed
final cached-source validation for 22 seconds. The 20-second supervisor stopped
the mount, but the read returned `Ok("shared")`; the immediately subsequent read
returned `Interrupted`. The original probe failed with exit 101.

The host now checks active operation state immediately before admitting a terminal
stage to the cache and after final validation/native I/O on every successful
exchange, before clearing its deadline. Source identity validation, integrity
verification and inherited OS resource policy remain intact. The corrected paths
are `session.rs:376`, `session.rs:402` and the private guard at `session.rs:419`.

`src/session_completion_tests.rs` exercises the actual `Mount::read`/`Mount::list`
exchange with real protocol frames and private spools. Its source validator
deterministically sets the supervisor stop flag or cancels the operation at final
validation. Each test covers cached V2 content, newly staged V2 content, legacy V1
content, V2 listings and V1 listings. Staged content is tested both immediately
before cache admission and after native spool readback. New stages must not enter the cache after
this stop/cancel point. The initial tests failed on both late successful content
and cancellation-time cache admission; the corrected tests pass without sleeps.
The original real-worker probe was also rerun: after 22.0018 seconds the read now
returns `Interrupted`, as does the subsequent read (exit 0).

## Review coverage

| Boundary | Evidence inspected |
| --- | --- |
| Integrity before publication | ZIP reader drained through CRC completion; 7z selected stream and discarded solid siblings drained through block verification; compressed streams checked through trailers/concatenated frames before sealing. Installed corrupt ZIP/7z cases returned no member bytes. |
| Stage authority | Host-issued stage IDs, active request/member binding, directional message checks, no pending/orphan member at terminal replies, and cross-request/forged-stage rejection tests. Reuse of sealed scratch within the same mount is intentional. |
| Offsets and lengths | Checked append/terminal ranges, native offset bounds, exact source-read length, bounded broker chunks and tests using `u64::MAX`, mismatched lengths, truncated data and incorrect offsets. |
| Ownership and cleanup | Native private spool handles, error poisoning, partial-write/flush failures, active lease survival after cache eviction, supervisor cleanup after in-flight I/O and cancelled/changed-source installed-worker tests. |
| Cache authority | Captured source revalidation before and after content I/O; changed-source rejection also applies to sealed cached content. The completion regression adds the previously missing stop/cancel guard. |
| Memory and large members | Production broker/storage paths use bounded chunks and disk stages. The fixture-only memory storage is separate. Virtual five-GiB TAR indexing and the real 180,858,888-byte ZIP member exercise paths beyond the former limits. No JPEG-specific path was found. |
| Native policy and backend boundary | Linux no-additional-CPU policy leaves inherited rlimits intact; Windows nested job setup does not request breakaway. Memory containment remains. Native operations and spool cleanup stay in backend services. |

The original logs were inspected, not inferred from the implementation report:
filesystem API has 11 passing tests per OS; the plugin has 36 ordinary passing
tests per OS; final installed-host logs have 21 passing tests per OS, with no
ignored host tests. The three manual plugin cases are two performance measurements
and a fixture exporter; the exporter was run separately. The old intermediate
host failure is superseded by the original final host logs.

Installed-worker evidence includes all eleven supported formats and the actual
86,013,866-byte user ZIP: 347,418 source bytes for index/listing, 85,983,232 source
bytes for the selected member, and 180,858,888 verified member bytes on disk.
Subsequent overlapping/tail reads use the cache without compressed-source rereads.

## Qualification after the correction

| Check | Windows | Native Linux |
| --- | ---: | ---: |
| Host unit tests | 16 passed | 16 passed |
| Discovery integration | 1 passed | 1 passed |
| Installed streaming worker | 3 passed | 3 passed |
| Existing installed worker | 3 passed | 3 passed |
| Total host tests | 23 passed, 0 ignored | 23 passed, 0 ignored |
| Strict host Clippy, all targets | Passed | Passed |

Host rustfmt check passed. After adding the extra post-read validation case, both
completion tests and strict all-target host Clippy were rerun successfully on each
OS (`completion-final-{windows,linux}.log`); production code was unchanged.
Commands used the existing dedicated
`_build/archive-streaming-host/{windows,linux}/target`, locked offline dependencies,
and at most two compiler jobs. No new dependency, API package or worker binary was
required by the correction. No root application target was used.

Windows command (PowerShell, protected test installation access):

```powershell
& _build/archive-streaming-implementation/final-host-windows.ps1 2>&1 |
    Tee-Object _build/archive-streaming-review/final-host-windows.log
```

Native Linux command from PowerShell:

```powershell
wsl -d Ubuntu-24.04 -- bash -lc 'set -o pipefail; bash /mnt/d/experiments/dev/FM/_build/archive-streaming-review/final-host-linux.sh 2>&1 | tee /mnt/d/experiments/dev/FM/_build/archive-streaming-review/final-host-linux.log'
```

The copied Linux runner differs only in where it records its temporary installed
worker location, preserving the original evidence. Both runners and the real-worker
probe commands are retained under `_build/archive-streaming-review/`. Baseline
`source-hashes.json` and baseline logs were not rewritten. The review directory's
new `source-hashes.json` identifies changed/new source and evidence hashes, and
`commit-slice.json` plus `commit-whole-files.txt` distinguish whole-file candidates
from files requiring partial staging. They are review aids, not git operations.

## Remaining runtime limits

There is no artificial production source/member/aggregate-file-byte or expansion
ratio ceiling. Native file-offset limits, available disk, worker memory, metadata
count/name budgets and bounded codec windows still apply. The 100,000-node and
8 MiB retained-name limits are not universal preallocation limits: pinned parsers
may allocate metadata before the project can reject it. TAR GNU/PAX extension
records, ZIP central metadata and decoded 7z headers remain subject to the worker
process memory boundary. Malformed-metadata rejection at each smaller allocation
boundary was not newly qualified by this review.

CRC verification may require full selected-member or solid-block decoding before
the first preview byte. A genuine long-running operation can continue while making
progress; existing OS CPU limits still constrain it. Source identities are live
validated, not immutable snapshots. A blocked OS/source call can delay the return
of an interrupted operation and final cleanup until that call returns; the corrected
completion guard prevents its late result from succeeding. Native primitive support
(including Linux `O_TMPFILE`) remains required. No real user volume was filled;
disk-full evidence uses deterministic fault injection and `/dev/full`.

The new tests and host correction do not qualify unrelated UI behavior, PFS02
folder-size results, or a reconstructed isolated commit. The shared tree contains
other edits; the integrator must preserve prerequisites and recheck the final
selected commit slice.
