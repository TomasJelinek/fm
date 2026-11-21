# Transfer and listing performance — 21 November 2026

## Scope

The measured items of the 20 November performance handover, applied on top of
`ca79fc6d` and measured before and after:

1. `DirectoryIndex::with_capacity` reserved a native-path map for every row, also for
   archive and Network Neighborhood rows, which have no native path.
2. `ArchiveExtensionHints` kept each file row's suffix in a heap `String`.
3. Linux collation keys took 4 bytes per `wcsxfrm_l` weight (312 bytes for the en_US key
   of a 16-character name).
4. Same-volume native Move synced four folders (two journal commits) and Copy and Move
   walked folder ancestry (`parent_key`) about 10 times per copied file and 40 times
   per moved file, with four statx calls per level.
5. Copy syncs the destination folder after every published file. Batching that sync is
   designed below but not implemented (see
   [Copy folder sync batching](#copy-folder-sync-batching-design-not-implemented)).

Storing an archive member's display name once is out of scope (an API change for the
Architect).

## Changes

| Item | Change | Files |
| --- | --- | --- |
| 1 | `with_capacity` reserves no native-path map; `push` reserves the rows still expected (`file_sizes` capacity minus rows pushed) when the first native path arrives | `midnight-core/src/entry.rs` |
| 2 | A folded suffix of up to 14 bytes is stored inline (`None` / 14 bytes and a length / a boxed longer suffix): the hint takes 16 bytes instead of 24 and allocates nothing, and `Entry` is at most 240 bytes instead of 248 (Windows; 232 on Linux). Short ASCII suffixes fold to ASCII lowercase without a temporary string, as `UniCase` does. `Debug` output is unchanged | `midnight-core/src/archive_extensions.rs` |
| 3 | Each weight is written in an order-preserving, prefix-free variable-length code: 1 byte for 0–126, 2 bytes up to 30,846, 3 bytes up to 358,526, 4 bytes up to 17,135,742, 5 bytes for the rest of u32 and (signed `wchar_t`) for negative weights. Keys are allocated exactly; an encoded key above 4 MiB is refused like a native output above it | `midnight-platform-linux/src/sort_keys.rs` |
| 4a | After a published native Move, the empty stage is removed first, then the source and target folders are synced once each (2 folder fsyncs and one ext4 journal commit instead of 4 fsyncs and two commits). A stage that cannot be removed is synced with both folders and reported as before; a replacement's stage, which keeps the displaced entry, is synced with both folders as before | `midnight-platform-linux/src/transfer_native_move.rs`; `transfer_recovery.rs` gains `remove_empty_stage_unsynced` |
| 4b | `parent_key` takes one statx per level (inode, device and mount ID) and reuses each folder's values from the level below, instead of separate identity and mount reads of each folder twice and a reopen of `.`; pin capture and refresh read their folder bindings with the same single statx | `midnight-platform-linux/src/transfer_namespace.rs` |
| 4c | A refresh batch (`TransferNamespaceRefreshBatch`, thread-local): the engine refreshes the pins of a change as one batch and the Linux announcement reserves its pins in one; within a batch the Linux adapter walks the ancestry of each folder once (keyed by the folder's identity and mount) | `midnight-platform-common/src/transfer_namespace.rs`, `midnight-engine/src/copy_namespace.rs` (one guard), `midnight-platform-linux/src/transfer_namespace.rs` |

## Method

- Machine: Windows 11 host with WSL 2 Ubuntu 24.04.4 (kernel
  6.6.87.2-microsoft-standard-WSL2, glibc 2.39), 20 logical CPUs, 15.4 GiB; ext4
  (`data=ordered`) on the WSL virtual disk, which also holds the tests' `TMPDIR`. Other
  agents' builds ran on the same host at the same time; see
  [Timing](#timing) for how this was handled.
- Debug test profile (`cargo test`, debug info off), as for the handover's numbers.
  Each variant's test executables were built once in a separate clone and kept:
  `before` (`ca79fc6d`), `after1` (items 1, 2, 4a and 4b, with a first version of item 3
  that the transfer timings do not use), `after3` (all items), and measurement-only
  variants of those described below. Runs of the variants alternate,
  so drifting load affects all of them alike.
- Timing: `copy_operations::tests::timing::native_move_of_one_file` and
  `copy_of_many_small_files` (`MIDNIGHT_TIMING_MOVES`, `MIDNIGHT_TIMING_FILES`,
  `--ignored --nocapture --test-threads=1`).
- Folder syncs were counted by an `LD_PRELOAD` library that wraps glibc's `fsync` and
  `fdatasync` and prints the totals at exit (the standard library's `sync_all` calls
  glibc's `fsync`).
- Ancestry walks were counted, timed and attributed to their callers by a
  measurement-only patch of the clone (a guard at the top of the walk, a backtrace per
  walk when asked), built as `before-count`, `after2-count` (all items but 4c) and
  `after3-count`. Neither tool is part of the change.
- Memory: `providers::archive::tests::archive_folder_memory_per_member` (one process per
  case, RSS growth for 1,000,000 members with 16-byte names and the stand-in lowercase
  keys) and the new ignored `sort_keys::tests::collation_key_sizes` (one process per
  encoding, RSS growth for 1,000,000 real keys of `file-0000001.txt`-style names, with
  `LOCPATH` pointing to en_US, cs_CZ and de_DE locales compiled with `localedef`).
- Key timing: `sort_keys::tests::collation_key_timing` (100,000 en_US keys, best of 5
  rounds in one process), before and after executables alternating.

## Results

### Archive panel memory

Bytes per member, two runs per variant (the runs agreed within 1 byte):

| Case | Before | After |
| --- | ---: | ---: |
| Host listing rows | 186 | 186 |
| Panel listing (rows, keys, index) | 649 | 639 |
| Panel listing with the file rows' suffix hints | 681 | 639 |
| Tree provider capture | 338–339 | 338–339 |

The panel saves the 8 bytes of `Entry` and the 2 bytes of control bytes per member that
the unused native-path map wrote. Its reservation also took about 64 MiB of untouched
address space for a million rows (computed: 2^21 buckets of 32 bytes), which RSS does
not show. Suffix hints no longer cost
anything for suffixes of up to 14 bytes (they cost 32 bytes per member).

### Linux collation keys

| Locale, 1,000,000 keys | Key bytes per name, before → after | Process growth per name, before → after |
| --- | --- | --- |
| en_US.UTF-8 | 312 → 108 (−65.4%) | 346 → 154 bytes |
| cs_CZ.UTF-8 | 312 → 108 (−65.4%) | 346 → 154 bytes |
| C.UTF-8 | 68 → 17 (−75%) | 121 → 58 bytes |

The process growth includes each key's 24-byte `Vec`. A panel holds the key in the same
way, so with real en_US keys an archive panel member is about 192 bytes smaller as well
(a 320-byte allocation becomes a 128-byte one); the panel measurement above uses
stand-in keys and does not show this. Over 43,713 distinct real file names (every name
under `/usr`), en_US keys shrink from 330.9 to 114.6 bytes per name and C.UTF-8 keys from
72.2 to 18.1. glibc's largest en_US weight of any single character is 65,955 (U+9FA5),
so no en_US weight takes more than 3 bytes, and no code point more than 4.

Key preparation got faster in the debug profile: 12.08 → 8.91 µs per en_US key (medians
of 5 alternating runs, faster in all 5; ranges 10.22–12.55 and 7.33–9.30). A first,
table-driven version of the code was 3.7 times slower than before (12.7 → 46.5 µs per
key) and was replaced by straight-line arithmetic before measuring.

### Folder syncs and ancestry walks

Counts per operation, deterministic across runs (20 Moves; 200 files of 125 bytes into
one folder; the fixtures lie 11 levels below `/`):

| | Before | After (all but 4c) | After (all) |
| --- | ---: | ---: | ---: |
| Folder fsyncs per Move | 4 | 2 | 2 |
| Ancestry walks per Move | 40 | 40 | 36 |
| fsyncs per copied file (stage file, destination folder) | 2 | 2 | 2 |
| Ancestry walks per copied file | 10.2 | 10.2 | 8.2 |
| Syscalls per walked level | 5 (4 statx, 1 open) | 2 (1 statx, 1 open) | 2 |
| Syscalls per walk from a folder 11 levels below `/` (12 levels) | 63 | 27 | 27 |

Time spent walking (medians of 3 alternating runs of each counting variant):

| | Before | After (all but 4c) | After (all) |
| --- | ---: | ---: | ---: |
| Per Move | 1.86 ms | 1.72 ms | 1.13 ms |
| Per copied file | 0.64 ms | 0.39 ms | 0.30 ms |

The walk itself took about 16% of a Move's process CPU time and 22% of a copied file's
before the change. The per-walk times are noisier than the counts (the host was busy, see
[Timing](#timing)); the syscall counts are exact.

Where the remaining walks come from (after, from backtraces): per copied file, 3.2 in
the engine's refresh of the publication's pins (4.2 before; a batch walks each folder
once, so these pins sit in about three different folders), 3 in the reservations of the
publication (4 before; one of them, the new name's own reservation, comes before the
announcement's batch), 1 in the capture of the new name's private pin and 1 in the
announcement. Per Move, 8 in the engine's refreshes (unchanged: no two pins of one
refresh share a folder), 13 in the engine's revalidations of the source pin (separate
calls at different times, so not one batch), and the rest in single captures,
announcements and bindings.

### Timing

Other agents built on the same host throughout: the host CPU load sampled every 10 s
during the two timing campaigns had medians of 60% and 52% (13–100%). The tables
therefore pool the two campaigns' alternating runs (18 runs of 100 Moves, 12 runs of a
Copy of 1,000 files of 125 bytes per variant) and add the median ratio of each run to the
`before` run beside it, which cancels slow drifts of the load.

| Same-volume Move of one file | Before | After | Paired ratio after / before |
| --- | ---: | ---: | --- |
| Median of the runs' median Move | 13.88 ms (7.97–47.35) | 9.54 ms (6.44–25.84) | 0.73, faster in 18 of 18 |
| Process CPU per Move | 11.70 ms | 9.25 ms | 0.82 |

The variant with items 1, 2, 4a and 4b only (9 runs) took 10.44 ms and 9.70 ms CPU per
Move (paired ratio 0.79, faster in 9 of 9): most of the gain comes from the fsync order
and the cheaper walks, the rest from fewer walks.

| Copy, per file of 125 bytes | Before | After | Paired ratio after / before |
| --- | ---: | ---: | --- |
| Wall time | 4.57 ms (3.21–6.06) | 4.00 ms (2.97–6.57) | 0.94, faster in 9 of 12 |
| Process CPU | 2.88 ms | 2.44 ms | 0.91 |

The first, sequential campaign at a host load near 100% gave the same direction with
far larger spread; its timings are not used. The handover measured 41–57 ms per Move and
24 ms per copied file on another Linux machine whose journal commits were much slower;
this virtual disk's fsync is fast, so the fsync reduction shows less here than it would
there.

## Copy folder sync batching: design, not implemented

Copy syncs the destination folder after each publication (the second of its two fsyncs
per file). Batching it changes the transfer contract and progress reporting, so it is
left as a design:

- It needs a core API change, an Architect decision: a publication that skips the folder
  sync (a new `StagedTransferFile` method whose default publishes as today, returning a
  distinct "published, sync pending" outcome so no consumer can mistake it for a durable
  publication) and a `TransferDirectory` call that syncs the names published into the
  folder (Linux: fsync of the held folder; Windows and other adapters: nothing, as today,
  since Windows promises no parent-entry durability). Five consumers handle publication
  outcomes (`copy_worker`, `copy_link_worker`, `file_list_output`, `preferences_file`,
  `verified_output`).
- Only plain Copy publications without replacement would defer the sync. Move keeps the
  per-file sync, because a Move proof requires every durability step before the source is
  removed; replacements, links and folders keep theirs.
- The engine keeps the pending publications of one folder and counts them in
  `files_completed`, `completed_bytes` and the work budget only after the folder's sync.
  It syncs after 64 files, 8 MiB or 250 ms since the first pending file, before
  publishing into another folder or creating one, before any conflict or error prompt,
  before a pause takes effect, before Retry, on cancellation and before terminal
  progress.
- A failed sync counts the batch's files as completed (they are published), records one
  error ("Copied N files to <folder>; durability is unconfirmed: …") and stops the job, as
  `PublishedDurabilityUnconfirmed` does for one file today.
- Tests: a fake destination counting syncs (N files give ⌈N/64⌉ syncs, plus one at each
  folder change, prompt, pause, cancellation and job end, each before its event);
  `files_completed` lags until the sync; a failed sync gives one error, stops the job and
  counts the batch; Move and replacements still sync per file; on Linux an injectable sync
  shows one folder fsync per batch; the timing test before and after.
- Gain: a measurement-only variant of the Linux adapter that never synced the folder after
  publication (all other changes included) took 3.28 ms and 2.37 ms CPU per copied file
  against 4.00 ms and 2.44 ms (paired ratio 0.77, faster in 11 of 12 runs). That is the
  most batching could save here, about a fifth of the wall time; on a disk with slow
  journal commits, where the handover found the folder sync in half of the publishing
  time, it would save more.

## Tests

New tests pin the behaviour and guarantees of each change:

- `entry::tests::native_path_map_is_reserved_only_when_a_native_path_arrives`: provider
  rows reserve no map; the first native row reserves the remaining rows, with no growth
  after it; an index without a size still works.
- `archive_extensions::tests`: suffixes of 1–40 bytes keep their text, inline up to 14
  bytes and on the heap above (`retained_bytes`), and the hint is 16 bytes; the stored
  suffix equals `UniCase` folding for ASCII, the Kelvin sign, ẞ, Czech, CJK, combining and
  long suffixes; the `Debug` output keeps its former form.
- `sort_keys::tests`: every class boundary, the wchar_t and u32 extremes and negative
  weights keep their order and no code is a prefix of another; 400 random weight
  sequences (160,000 pairs) order exactly as their former fixed-width keys; 76 real names
  in C, C.UTF-8 and, where available, en_US, cs_CZ and de_DE keep the fixed-width order,
  equal the two-pass transform and never grow; C keys take one byte per ASCII character
  and are allocated exactly; an encoded key above 4 MiB is refused. The former test of the
  4-byte encoding was replaced.
- `transfer::namespace::tests`: the ancestry key lists every live ancestor's identity and
  mount, as separate reads of each ancestor give, and follows a moved ancestor; a refresh
  batch walks a folder shared by two pins once, a nested batch walks for itself and an
  ended batch is forgotten.
- `transfer::move_source::native::tests`: a published Move syncs exactly the source and
  target folders once, after the stage was removed; a failed sync after the removal
  reports the published Move for recovery and leaves no stage; a stage that is no longer
  empty is synced with both folders and reported; a replacement's stage is synced with
  both folders and kept.

Windows, in the shared worktree (with other agents' uncommitted changes), target directory
`_build/agent-p`:

- `cargo test -p midnight-core --lib --locked --offline -- archive_extensions entry::`:
  11 passed.
- `cargo test -p midnight-core --test integration --locked --offline -- archive_ tree_view listing_performance selection`:
  53 passed, 2 ignored.
- `cargo test -p midnight-platform-common --lib --locked --offline -- transfer_namespace`:
  6 passed.
- `cargo test -p midnight-engine --lib --locked --offline -- copy_namespace`: 55 passed.

Linux, in a clone of `ca79fc6d` with only these changes (the test executables of
`cargo test --no-run`, run with `--test-threads=8`):

- `midnight-core` library, all tests: 122 passed (118 before).
- `midnight-platform-linux` library, all tests: 431 passed, 1 failed, 34 ignored (before:
  421, 1, 33). The failure,
  `desktop::user_command_tests::custom_posix_terminal_transports_raw_environment_without_evaluating_filename_source`,
  fails the same way before the change in this WSL environment.
- `midnight-platform-linux`, `sort_keys:: transfer::` with `LOCPATH` holding compiled
  en_US, cs_CZ and de_DE locales: 181 passed, 6 ignored; the ignored Czech key test with
  them: 1 passed; `sort_keys::` without them: 10 passed, 3 ignored.
- `midnight-engine` library, `copy_namespace copy_operations:: providers::archive transfer`:
  489 passed, 6 ignored.

## Not measured or not changed

- Absolute numbers depend on the disk: this virtual disk's fsync is fast, so the
  handover's multi-second Move stalls and 24 ms per copied file did not occur here; the
  counts above (fsyncs, journal commits, walks, syscalls) carry over.
- Release builds were not measured.
- The engine's 13 revalidations of a moved source's pin per Move are separate checks at
  different times; joining them is a separate design question.
