# Archive opening and navigation performance — 26 November 2026

## Scope

Measures how much it costs to open and navigate a large archive: about 100,000 small
files in about 1,000 folders. Where a measurement showed a real gain, the work is made
cheaper without changing behaviour or any bound. This builds on the
[archive opening performance](2026-11-08-archive-opening-performance.md)
record (TAR/7z eager decoding removed, worker startup and source transfer named as the
remaining costs) and the
[transfer and listing performance](2026-11-21-transfer-and-listing-performance.md)
record (host and panel bytes per member).

Commits on main (after `1f7a85ba`):

| Commit | Change |
| --- | --- |
| `10f83875` | Measurement only: plugin `performance.rs` `measure_many_small_members`, host example `measure_navigation` |
| `4ca06cee` | Archive worker: resolve a member's folder once per run of names in that folder (`tree.rs`, `lib.rs`; two new tests in `tests/archives.rs`) |
| `5406c443` | Archive worker: index ZIP names straight from the zip entry (`formats.rs`) |

Only the independent archive plugin (`plugins/midnight-fs-archive`) changed, plus one
host example. The plugin was built here against a public worker SDK bundle that the
repository's own packager (`midnight-api-package`, `worker-packages`) produced under
`_build`. No host product code, published API, dependency or bound changed.

## Method

- Machine: Windows 11, 20 logical CPUs. Release profile. Other agents' builds ran on
  the same host for part of the time (see [Noise](#noise)).
- Fixtures were written by a Python `zipfile`/`tarfile` script under `_build/tmp`:
  100,000 files of 16–2,047 bytes in 1,000 leaf folders, three levels deep
  (`dir-NN/sub-NN/leaf-NN/file-NNN.txt`). With explicit folder records there are
  1,111 folders and 101,110 members.
  - `many.zip`: deflated, 25.1 MB, with an 8.1 MB central directory.
  - `many.tar.gz`: 10.6 MB, which decompresses to 179.9 MB of TAR.
  - `many.7z`: 8.9 MB, written by the test from the ZIP's files with sevenz-rust2,
    one solid block per folder.
- **In-process** (`cargo test --release --test performance measure_many_small_members
  -- --ignored`): `Archive::open` on an owned buffer, the root listing, and a walk down
  the last folder of each level and back up again, listing every folder as the pane
  does. Each run also hashes every listed node (ID, parent, names, kind, size, time,
  readability) into a tree digest, so two builds can be compared.
- **End to end** (`cargo run --release -p midnight-filesystem-host --example
  measure_navigation`): the real sandboxed worker (AppContainer and IPC), with a new
  mount per run. It times:
  - first listing: Open, the root listing, and the pane's rows, sort and index;
  - down and back: 7 listings, each with its pane rows;
  - a size sort of the deepest folder and of the root;
  - listing all 1,111 folders, which is what an archive folder-size scan requests;
  - release of the mount.
- Each binary ran 5 runs per round and reports their median. Before and after binaries
  or installed workers alternated round by round (5–7 rounds). The tables give the
  median of the round medians and the median of the paired after/before ratios.
- Worker memory: the example holds the last mount open (`MEASURE_HOLD_MS`) while
  PowerShell reads the worker process's private bytes and peak working set (one
  sample per archive).
- Phase split: temporary `eprintln!` timers in a scratch build, not committed.

## Results

### Baseline: where the time goes (before the changes)

| | ZIP | tar.gz | 7z |
| --- | ---: | ---: | ---: |
| In-process Open | 111.2 ms | 217.1 ms | 84.9 ms |
| End to end, first listing | 226.1 ms | 433.1 ms | 106.7 ms |
| End to end, down and back (7 listings) | 6.2 ms | 5.9 ms | 1.4 ms |
| End to end, all 1,111 folders | 1.08 s | 1.08 s | 0.19 s |
| Size sort of 100 + 10 rows | < 12 µs | < 12 µs | < 8 µs |
| Worker private bytes while mounted | 56.3 MiB | 12.5 MiB | 26.3 MiB |
| Worker peak working set | 70.6 MiB | 17.2 MiB | 43.4 MiB |

- **Worker startup:** the first listing of a one-file ZIP takes 20.3 ms (median of 7).
- **In-process listings** of a folder of 100 members take under 50 µs. The worker
  serves listings from its index; navigation cost is all exchange overhead.
- **ZIP Open, in process (111 ms).** The zip crate's central-directory parse and its
  `by_index_raw` scan take 42 ms. That scan also reads every local header, because
  `by_index_raw` locates each entry's data. The project's record walk takes 13 ms. The
  entry loop takes 61 ms, of which `Archive::add` is 38 ms: three temporary vectors per
  name and an allocated `(parent, name)` key for every folder lookup, including hits.
- **7z Open, in process (85 ms before, 64 ms after the first change).** After the first
  change, `sevenz_rust2::Archive::read` takes 45 ms. The index loop takes 12 ms and
  `Content::new` 6.5 ms.
- **tar.gz Open, end to end** is dominated by decoding and staging 180 MB of scratch
  TAR through the host. That is the protocol's design.
- **Per-exchange cost.** Every exchange (each listing page, each source read)
  revalidates the captured source by reopening the file and comparing its revision
  snapshot. On this machine, reopening `many.zip` or `many.tar.gz` costs about 1 ms;
  `many.7z` costs 0.085 ms (1,000 opens timed from PowerShell), and a one-file ZIP is
  also cheap. Endpoint scanning of large ZIP/gzip files on open is the likely cause; it
  was not isolated. This is why listing every folder costs about 1 ms per folder for
  ZIP and tar.gz but 0.17 ms for 7z. It also explains why ZIP source reads cost about
  0.53 ms per 256 KiB window against 0.16 ms for 7z (`measure_open`: full-source read
  51 ms against 5.5 ms).

### Changes

**1. Folder of consecutive names (`4ca06cee`).** The index remembers the raw folder
part of the last name and the folder it resolved to. A name with the byte-identical
folder part, whose final component is not `.`, uses that folder directly. Every other
name takes the unchanged general walk. The same raw folder part always resolves to the
same folder, because indexing only adds folders.

| | Before | After | Paired ratio | Rounds faster |
| --- | ---: | ---: | ---: | ---: |
| In-process Open, ZIP | 111.2 ms | 86.5 ms | 0.782 | 5/5 |
| In-process Open, tar.gz | 217.1 ms | 188.7 ms | 0.869 | 5/5 |
| In-process Open, 7z | 84.9 ms | 63.8 ms | 0.739 | 5/5 |
| End to end, first listing, ZIP | 226.1 ms | 196.6 ms | 0.873 | 5/5 |
| End to end, first listing, tar.gz | 433.1 ms | 413.4 ms | 0.937 | 4/5 |
| End to end, first listing, 7z | 106.7 ms | 85.1 ms | 0.797 | 5/5 |
| End to end, down and back | 6.2 / 5.9 / 1.4 ms | 6.2 / 6.1 / 1.4 ms | 1.02 / 1.05 / 0.99 | unchanged |

`add()` inside the ZIP entry loop fell from 38 ms to 14–17 ms. Worker private bytes
were 56.3 → 56.7, 12.5 → 12.9 and 26.3 → 26.7 MiB (one sample each; the remembered
folder part is a single name, at most 64 KiB, and is released after indexing). Peak
working set was unchanged within 0.2 MiB.

**2. ZIP names without temporary copies (`5406c443`).** Each ZIP entry's raw and decoded
names are passed straight from the zip entry to `Archive::add`, which copies them into
its arena anyway. This removes 2 heap allocations and copies per entry: 200,000 for
these fixtures.

| | After 1 | After 2 | Paired ratio | Rounds faster |
| --- | ---: | ---: | ---: | ---: |
| In-process Open, ZIP | 95.4 ms | 81.6 ms | 0.857 | 7/7 |
| In-process Open, tar.gz / 7z (controls, untouched) | 207.0 / 65.0 ms | 211.5 / 66.4 ms | 1.02 / 1.02 | 1/7, 1/7 |

The end-to-end ZIP first listing measured 228.8 → 222.9 ms (paired ratio 0.843, faster
in 5 of 7). Outside builds doubled some rounds during that campaign, so this figure is
indicative only.

### Behaviour

- The tree digests of all three fixtures are identical for the before, after-1 and
  after-2 builds (ZIP `bdcd9e91e1f7e556`, tar.gz `9c294250bc389180`,
  7z `e3a8d141f3d57f45`; 10, 14 and 14 runs per build in the three campaigns).
- The two new tests pass on both the old and the new code:
  - `members_of_consecutive_names_find_their_folders_however_spelled` covers `./a`,
    `a/./sub`, folder records, `.` endings, and unsafe names and top-level names
    between runs;
  - `cp437_members_of_one_folder_keep_their_decoded_labels` checks decoded labels of
    the second member of a folder.

### Bounds

No bound changed:

- protocol frame, batch, read and name limits (1 MiB, 128, 256 KiB, 64 KiB);
- the host's listing caps, exchange byte and time budgets, and per-exchange source
  revalidation;
- the worker memory policy and the index's `try_reserve` limit;
- declared-size budgets and member IDs.

### Noise

Other agents' builds ran during part of this work: 5–6 `cargo`/`rustc` processes and
up to 83% CPU. The change-1 campaigns ran while the host was quiet (spread within ±3%
for the unchanged binary). A final campaign that compared the original with the final
worker end to end ran under that load: the unchanged ZIP varied 223–476 ms between
rounds. That campaign is therefore not used. Its in-process half still showed ZIP
0.757, 7z 0.784 and tar.gz 0.966 paired ratios, with the same digests.

## Tests run

- Plugin, with the packaged worker SDK and its target under `_build` (each commit):
  - `cargo fmt --check`: pass;
  - `cargo test`: 96 passed, 0 failed, 7 ignored (94 passed before the two new tests);
  - `cargo clippy --all-targets -- -D warnings`: pass.
- Host:
  - `cargo fmt -p midnight-filesystem-host -- --check`: pass.
  - `cargo clippy -p midnight-filesystem-host --examples -- -D warnings`: pass.
  - `cargo test -p midnight-filesystem-host`: 137 passed, 0 failed, 12 ignored.
- Sandboxed worker, with the final worker installed under `_build`
  (`ARCHIVE_TEST_INSTALL_ROOT`, `ARCHIVE_WORKER`):
  - `real_sandboxed_worker_lists_reads_and_refuses_a_cancelled_open`: pass;
  - `real_workers_open_archives_inside_archives_and_give_every_level_up`: pass;
  - `real_sandboxed_worker_lists_archives_of_millions_of_members`: not run. Its
    generated fixtures (`MIDNIGHT_ARCHIVE_LARGE`) were not provided; it failed on the
    missing variable.
- Not run:
  - midnight-engine tests (`cargo test -p midnight-engine --lib -- archive` and the
    archive integration tests), because the engine is unchanged;
  - Linux/macOS, and the Linux-only `measure_index_memory`;
  - the acceptance suite.

## Left as is, and why

- **ZIP local headers read at Open.** zip 8.6's `by_index_raw` reads every entry's
  local header (`find_content`), so Open reads most of the archive, not just its
  central directory. The pinned API has no per-entry metadata accessor without that
  read. Avoiding it would also stop Open from rejecting a broken local header, which
  is a behaviour change.
- **The central directory is read twice**: once by the zip crate, once by the
  duplicate-record walk (8.1 MB, about 31 extra 256 KiB host round trips here).
  Keeping it in worker memory instead would raise the worker's peak memory by up to
  the directory's size, which changes the memory bound in practice.
- **Per-exchange source reopen** (about 1 ms per listing page or source read for
  large ZIP/gzip files on this machine). The engine's panel listing also calls
  `validate()` before and after each listing. These reopens are the source-revision
  security check. Weakening them is an Architect/Security decision, not a performance
  fix. They dominate listing every folder (about 1.1 s for 1,111 ZIP folders).
- **Third-party parse time**: zip about 42 ms, sevenz-rust2 about 45 ms. **tar.gz
  scratch staging**: 180 MB, by protocol design.
- **The ZIP record map** (13–15 ms: a boxed key per record, SipHash) could shed
  allocations with an arena, but an identical duplicate-name verdict needs an exact
  key set. The gain would be at most about 5% of the end-to-end figure, so it was not
  attempted.
- **Host-side rows, sort and index** for 100-member folders are microseconds. Per-member
  host and panel bytes are in the 21 November record and were not re-measured.

## Evidence

Logs under `_build/qa/` (not committed):

- in-process A/B: `nav-ab1.log`, `nav-ab2.log`, `nav-final.log`;
- end-to-end A/B: `host-ab1.log`, `host-ab2.log`, `host-final.log` (loaded);
- plugin checks: `plugin-test-*.log`, `plugin-clippy-*.log`, `plugin-fmt-*.log`;
- host checks: `host-tests.log`, `host-worker-tests.log`, `host-clippy.log`,
  `host-fmt2.log`.

Fixtures were removed after measuring.
