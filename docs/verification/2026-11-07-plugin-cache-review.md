# Shared thumbnail cache review

Multiple application instances share one cache directory. Each instance's memory
index is only a hint: another instance can publish, replace or remove records.
Clearing and disk-budget enforcement must use a current directory snapshot.

All cache transactions acquire an OS file lock on a private, persistent lock file
opened relative to the validated store root (or beneath pinned Windows ancestors).
The lock file is never replaced or removed. Separate opens give each transaction
independent ownership, including two caches in one process. Acquisition retries
for at most one second on backend workers and returns an explicit busy error.
The OS releases the lock if its owner exits. No UI thread acquires it.

The lock record contains a generation and its bitwise complement. New lock files
initialize once; malformed or interrupted generation records remain invalid and
force inventory rescans rather than reusing an earlier generation. Each mutation,
including an access-time update, advances the generation before touching records.
An instance tags its index only after successful completion. A worker crash after
the advance therefore leaves other instances with a mismatched index generation.

Open and clear refresh the bounded disk inventory while holding this lock. Put
refreshes only when another instance changed the generation or the generation is
invalid. Eviction and atomic publication stay inside the same transaction. Reads
acquire the lock without rescanning every record; newly observed disk hits update
local accounting. Clear enumerates current records, so a cache opened before
another instance generated previews still removes those previews. Warm memory
hits continue checking persistent-record existence before presentation.
Source revision validation runs before and after the transaction, so potentially
slow provider I/O never holds the cross-process cache lock.

Windows regression evidence: before the changes, two already-open caches left a
record behind after clear and accumulated 1086 bytes under a shared 600-byte
budget. Both regressions now pass, together with malformed-generation fallback,
independent transaction contention and actual cross-process lock exclusion.

The ignored `full_cache_generation_fast_path_cost` fixture measured an inventory
scan of 8192 records at 616.9 ms. One hundred full-cache puts with synchronized
generations took 716.1 ms total (7.16 ms per put), including eviction, file sync
and atomic publication. These are local unoptimized Windows measurements, not a
cross-platform performance guarantee. The benchmark creates only owned fixtures
under the configured `_build` test root.
