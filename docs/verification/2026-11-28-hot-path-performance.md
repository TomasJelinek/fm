# Hot path performance — 28 November 2026

A Perf pass over the operations users feel most on Windows (NTFS, local disk): panel
listing, sorting, selection and quick search, Find, folder sizes, checksums and Copy.
Each was measured first; two changes with measured gains were made, and the other
hotspots are recorded with their causes. Earlier records this builds on:
[listing metadata](2026-11-07-listing-performance.md),
[panel scrolling](2026-11-09-panel-performance.md),
[transfer and listing](2026-11-21-transfer-and-listing-performance.md) and the
[UI fixes](2026-11-24-ui-perf-fixes.md).

## Method

- Machine: Windows 11, 20 logical processors, NTFS on a local SSD; toolchain 1.99.1.
  Every number is from the release profile (fat LTO, as shipped) unless stated.
  Fixtures were made under the worktree's `_build` and deleted afterwards.
- Other agents shared the host. Each end-to-end run recorded the processor load and
  the number of Cargo, rustc and linker processes before it started: load 0–24%,
  no compiler process during any measured run.
- Before and after binaries were kept side by side and run alternately (before,
  after, before, …), five times each; tables give the median of the five, with the
  range. "Before" is `d71c9850`.
- Fixtures: `n1000`, `n10000` and `n100000`, folders of 1,000, 10,000 and 100,000
  entries (every tenth a folder; names numbered for natural sorting, in mixed case,
  with Czech and CJK names and ten extensions; files of 0–4,095 bytes), their
  modification times spread over three years.

### Harnesses

- `crates/midnight-explorer/examples/listing_probe.rs` (new): the production Engine,
  the installed native filesystem plugin (`InstalledProvider::shared_composition`, the
  package built by `scripts/build-filesystem-package.ps1` beside the example), the real
  `Presenter`, row models and main window on Slint's software renderer, pumped as the
  application's event loop is (up to 16 events or 3 ms, then one refresh). It times
  navigation to the first rows and to the applied `ListingFinished`, the four sort
  columns ascending and descending, Ctrl+A, invert, invert again, Deselect all and three
  quick-search keystrokes, and breaks each step down by event (first arrival and UI
  apply time), refreshes and draws. `components` mode times the listing's parts one by
  one: bare enumeration, both providers, time text, collation keys, sorts, the index and
  the maps behind it. The software draw stands in for a GPU frame; its time is reported
  apart.
- `midnight-engine` `src/hot_path_measure_tests.rs` (new): Find by name and by content
  over a tree of 20,000 files; `folder_sizes::native_tests::native_size_of_a_deep_tree`
  (new); `checksum::worker::tests::native_large_file_throughput` (new); and
  `measure_one_large_file` (new) beside the existing small-file and 1 MiB-file Copy
  measurements. Commands and variables are in the
  [manual measurements table](../developer/test-execution.md#manual-measurements-probes-and-traces).
- `midnight-platform-windows` `locale::tests::listing_locale_timing` (new): the native
  formatting of modification times and collation keys per row, for 100,000 rows.
- A temporary, environment-gated timing trace in `list_directory`, `publish_listing`
  and the quick-search worker gave the per-phase split below (one run); it is not part of
  the change.

## Panel listing

End to end, `listing_probe measure`, after the changes (before in brackets where the
change is measurable):

| Folder | First rows in the model | `ListingFinished` applied | UI apply / refresh / software draw, whole listing |
| --- | ---: | ---: | --- |
| 1,000 | 1.3 ms (1.4) | 5.8 ms (5.9) | 0.4 / 1.4 / 8.5 ms, 16 refreshes |
| 10,000 | 1.5 ms (1.2) | 55.6 ms (55.9) | 3.5 / 6.7 / 18.0 ms, 91 refreshes |
| 100,000 | 1.4 ms (1.3) | 524.4 ms, range 517.7–541.1 (542.8, range 513.9–965.6) | 32.8 / 49.3 / 126.2 ms, 606 refreshes |

The first rows reach the row model about 1.3 ms after Navigate at every size; the
listing streams in batches of 128. The UI thread is busy for about a third of a 100,000-row
listing (apply 33, refresh 49, software draws 126 ms) and is never what the listing
waits for: the engine's listing thread is the long pole.

### Where the 100,000-row listing goes

The listing thread (before the changes, one traced run; the listing then took 600 ms):

| Phase | Time |
| --- | ---: |
| Reading the folder through the installed plugin, with the per-row work below | 478.6 ms |
| — of which: time text (`localize_metadata`) | 70.6 ms |
| — the streaming selection index (`StreamingSelections::append`, path copies into maps that grow) | 44.1 ms |
| — the copy of each row kept for publication (`entries.push(entry.clone())`) | 26.8 ms |
| — icon types, sending the batches | 3.0, 1.8 ms |
| Collation keys (`prepare_sort_keys`, one `LCMapStringEx` per name) | 50.4 ms |
| Sort | 17.1 ms |
| `DirectoryIndex::from_entries` | 35.0 ms |

The parts one by one (`listing_probe components`, medians of 5 rounds, two alternating
runs per variant; before → after):

| Part, 100,000 entries | Before | After |
| --- | ---: | ---: |
| `std::fs::read_dir` and `DirEntry::metadata` alone | 25.0–26.5 ms | 25.1–25.7 ms |
| Platform provider `read_directory` (also formats time text) | 150.9–154.0 ms | 124.4–127.2 ms |
| Installed plugin `read_directory` (the panel's provider) | 278.3–285.4 ms | 281.2–285.9 ms |
| Formatting every modification time | 60.4–60.6 ms | 40.2 ms |
| A copy of the localized rows | 25.1–27.5 ms | 25.4–26.5 ms |
| Collation keys (with one copy of the rows) | 71.6–72.5 ms | 69.9–71.4 ms |
| Sort Name / Size / Modified / Type, keyed rows | — | 5.3–6.7 / 7.4–7.5 / 5.5–5.8 / 13.0–14.9 ms |
| `DirectoryIndex::from_entries` | 28.6–30.8 ms | 27.1–27.6 ms |

The installed plugin's listing costs about 255 ms above the bare enumeration for
100,000 entries (2.6 µs per entry), against about 100 ms for the platform provider
without its time text. That difference lies in the filesystem SDK and wire protocol
(each entry is bound as a node in the mount's registry, encoded with its full address
and decoded again by the host) and in the public API/SDK workspaces, which this pass
does not change. It is the largest remaining listing cost; see
[Not changed](#not-changed).

## Change 1: Windows time text (`midnight-platform-windows` `locale.rs`)

Each row's modification time was converted to local calendar fields with
`SystemTimeToTzSpecificLocalTimeEx` (about 210 ns) and formatted with
`GetDateFormatEx` and `GetTimeFormatEx` into UTF-16 vectors, then strings: three native
calls and four allocations per row. Now, per worker thread:

- the local offset of each recent UTC day is kept (2,048 direct-mapped slots) when the
  day's first and last millisecond have the same offset, that is when no daylight saving
  transition falls inside it; a row's calendar fields are then `FileTimeToSystemTime` of
  its time plus the offset. A transition day is converted natively row by row;
- the text of the last dates and times is kept by their calendar fields (512 slots each;
  a failure is never kept), so each distinct value calls NLS once;
- NLS text is converted to a string straight from the stack buffer.

The captured locale snapshot never changes within a process, so a kept text is what a
new call returns. `listing_locale_timing`, 100,000 rows, medians of 3 runs of 5 rounds:

| Spread of the times | Before | After |
| --- | ---: | ---: |
| Every row in another second of three years | 591–624 ns per row | 428–437 ns |
| Rows within one day | 577–586 ns | 238–239 ns |
| All rows in one second (an extracted archive) | 571–578 ns | 100–104 ns |
| Calendar fields alone, three years | about 210 ns | 45–48 ns |

The platform provider's listing of `n100000` took 17% less (table above), the engine's
time text for the panel 34% less (60.5 → 40.2 ms). End to end the 100,000-row listing
finished in 524.4 instead of 542.8 ms (medians; the ranges overlap, so the end-to-end
gain alone is within the spread). The fixture's times are spread over three years,
the least favourable case; a folder whose files share days gains up to the "one day"
row. Find results and `local_calendar` use the same conversion.

Tests: `reused_modified_texts_equal_native_formatting` compares 10,000 reused and new
texts with direct native calls; `local_calendar_fields_by_day_offset_equal_native_conversion`
compares the fields of every 433.789 s over three years, twice (the second pass from kept
offsets), with the native conversion, across this zone's daylight saving transitions;
`recent_texts_format_each_distinct_value_once_and_keep_no_failure` checks reuse,
replacement and that failures are not kept. `midnight-platform-windows` library: 469
passed, 31 ignored (debug); `locale` tests in release: 13 passed.

## Change 2: quick-search projection (`midnight-engine` `search.rs`)

The first keystroke of a quick search builds the listing's projection (the persistent
filter's rows and their index); later keystrokes reuse it. The build grew its maps from
empty (about 17 doublings for 100,000 rows) and then checked every source row once more
against the index. Now the maps are sized for every row (and shrunk when the filter
keeps fewer than half), and the check runs only when the filter left a row out: when
every row passed, each row's push already proved its id and path unique, so each maps
back to its own row and the check cannot fail.

| Measure | Before | After |
| --- | ---: | ---: |
| `measure_quick_search_keystrokes`, 200,000 rows, 7 keystrokes building the projection each time | 932.9 ms (897.3–949.2) | 469.3 ms (459.0–518.4) |
| The same keystrokes with the kept projection (unchanged path) | 221.7 ms (219.5–229.0) | 231.9 ms (227.9–236.3) |
| End to end, first keystroke on `n100000` | 71.1 ms (67.2–87.1) | 36.2 ms (35.6–41.7) |
| End to end, first keystroke on `n10000` | 16.4 ms (14.0–20.3) | 12.6 ms (12.2–13.8) |

The projection itself now takes about 34 instead of 102 ms for 200,000 rows. A new test,
`a_filtered_out_row_sharing_a_projected_identity_still_fails_the_projection`, checks that
a filtered-out row aliasing a kept row's id or path still fails the projection and that
the unfiltered projection indexes every row; `search::` tests: 25 passed.

## Sorting, selection and quick search (end to end, after)

| Step, 100,000 entries | Median (range) |
| --- | ---: |
| Sort by size / date / extension / name, ascending | 95.0 / 101.0 / 130.2 / 90.4 ms |
| The same, descending | 99.1 / 97.5 / 106.4 / 78.4 ms |
| Ctrl+A, callback / selection shown | 0.46 ms / 9.3 ms |
| Invert all, callback / shown | 0.56 / 9.5 ms |
| Deselect all, callback / shown | 0.47 / 8.9 ms |
| Quick search, later keystrokes "re", "rep" | 17.0, 16.0 ms |

Sorting was not changed (before and after agree within 5%). A re-sort of 100,000 rows
(traced): copying the shown rows into the new snapshot 25–46 ms, sort 13–32 ms, the new
index 32–42 ms, then the UI. The selection worker answers a bulk selection in
0.3–0.5 ms after the click; almost all of the "shown" time is the software-rendered
frame (8–10 ms) that follows. A cached quick-search keystroke computes in about 9 ms on
the worker (traced).

## Find, folder sizes, checksums and Copy (measured, not changed)

| Operation | Median of 5 (range) | Per item |
| --- | ---: | --- |
| Find by name `*.rs` over 20,221 entries (221 folders, 4,000 results) | 554 ms (468–728) | 27 µs per examined entry |
| Find by name `*.*` (20,221 results) | 901 ms (842–1,103) | 45 µs per result |
| Find content "needle" (20,000 files, about 90 MiB, 200 results) | 3,471 ms (3,264–3,607) | 172 µs per file |
| Folder size of a deep tree (9,841 folders, 49,205 files) | 762 ms (704–1,046) | 77 µs per folder |
| Folder size, 100 folders of 50 files (existing probe) | 18.6 ms (12.5–24.2) | |
| Copy, 5,000 files of 1–16 KiB (existing measurement) | 13.20 s (12.58–13.54) | 2.64 ms per file, 379 files/s |
| Copy, 500 files of 1 MiB (existing measurement) | 2.42 s (2.14–2.53) | 4.84 ms per file, 206 MiB/s |
| Copy, one file of 1 GiB | 2.23 s (2.12–2.44) | 458 MiB/s with SHA-256 verification |

Checksums of one 1 GiB file (in the file cache; a plain read runs at 6,094 MiB/s):

| Algorithm | Median MiB/s (range) |
| --- | ---: |
| CRC32 | 5,600 (5,174–5,699) |
| MD5 | 669 (653–679) |
| SHA-1 | 1,733 (1,683–1,815) |
| SHA-256 (the default) | 1,647 (1,612–1,690) |
| SHA-512 | 679 (664–694) |
| All five | 270 (265–274) |

- **Find** opens every examined child once (no-follow, relative to its folder's handle)
  and queries its identity and revision, and opens each published result again by path
  to revalidate it; content search opens a reader as well. That is 27 µs per entry
  against 0.25 µs for bare enumeration. The opens are the
  [Find contract's](../developer/find.md) identity guarantees, so they were left alone.
- **Folder sizes** cost about 77 µs per folder of the deep tree, the folder open and its
  identity checks; the files inside a folder cost little.
- **Checksums** run the selected algorithms one after another on one worker, so "all
  five" takes the sum of their times (3.79 s for 1 GiB; the slowest alone, MD5, takes
  1.53 s). Hashing them in parallel would bring it near the slowest; the
  [checksum contract](../developer/checksums.md) describes a serial worker, so this is a
  design question, not done here. SHA-256 alone (1.6 GiB/s) is faster than most disks.
- **Copy** of small files spends about 2.6 ms per file on staging, verification,
  durability and metadata; the folder sync batching designed in the
  [21 November record](2026-11-21-transfer-and-listing-performance.md#copy-folder-sync-batching-design-not-implemented)
  remains the open item. One large file copies at 458 MiB/s including the verification
  read.

## Data structures

Measured in `listing_probe components` (100,000 native rows, release):

| Structure | Cost | Verdict |
| --- | --- | --- |
| `DirectoryIndex::by_path`, `HashMap<PathBuf, EntryId>` | 14.6–15.2 ms presized, 23.6–24.8 ms grown from empty; copying the paths 3.9–4.4 ms; `Path::hash` 6.2–6.8 ms against 1.5–1.6 ms for SipHash of the same bytes | Kept. `Path::hash` hashes each component separately (paths that differ only in separators are equal), so a faster hasher cannot remove most of it and hashing raw bytes would change equality. Presized in `from_entries`; the quick-search projection now presizes too (change 2). |
| `DirectoryIndex::by_id`, `HashMap<EntryId, usize>` | 2.0–2.6 ms presized, 3.6–4.2 ms grown | Kept: ids are sparse across listings, so a vector by id does not fit. |
| Streaming selection index (`streaming_selection.rs`) | 44 ms of the listing thread (traced): path copies and maps that grow batch by batch | Kept: the row count is unknown while streaming; selection during streaming needs it. |
| `Vec<Entry>`, 240-byte rows | Copy 25–27 ms (five heap strings per row: path, name, display name, date, time); sort 5–15 ms | Kept. A re-sort copies every row once; sharing rows (`Arc<Path>`, interned names) would touch every consumer of `Entry`, an Architect decision. |
| Recent time texts and day offsets (change 1) | Direct-mapped `Vec<Option<(u64, …)>>`, 512 and 2,048 slots per thread | Chosen over `HashMap`: a fixed bound and one probe per row, no growth, no eviction bookkeeping. |
| Presenter `streaming_index`, `HashMap<EntryId, usize>` | Part of the 33 ms UI apply of 782 batches | Kept: about 2 ms of growth at most. |

## Not changed

- The installed plugin's per-entry cost (2.6 µs, about 255 ms per 100,000 entries) is in
  `filesystem-api` (SDK node binding, wire encoding and decoding), a public SDK workspace
  outside this pass; it is the largest remaining listing cost.
- The panel refreshes the whole window model after every pump turn (about 600 refreshes,
  49 ms, for 100,000 entries) and resets the row model when the streamed rows grow;
  neither delays the listing, which waits for the engine.
- Collation keys (about 470 ns per name, one `LCMapStringEx` each) and the re-sort's
  row copy and index were measured but not changed.

## Unverified

- No GPU or physical-display timings: draws are software-rendered stand-ins, and the
  probe starts with both panes at the fixture root (its startup wait ends after 10 s
  because both panes still report loading; the measured navigations then complete
  normally).
- Mask selection and the panel filter were not timed end to end (the mask dialog is a
  separate window the probe does not drive).
- Cold-cache (first read after boot) listing and Find were not measured; all numbers
  are warm.
- Network drives and Linux were not measured.
