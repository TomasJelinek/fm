# Copy and Move of 1,000,000 selected items (COPY-04) — 2026-11-23

## Scope

Copy and Move took at most 4,096 selected native items (Move 64), because preparation
opened and pinned every item before the job started and kept it until it was copied.
A selection of more than 64 items now streams
([selection sizes](../developer/TRANSFERS.md#selection-sizes)): the job keeps the paths and
about 100 bytes per item, reads and totals each item just ahead of the copy, and opens
each item only when it takes it. This record gives the numbers measured for 1,000,000
items. All runs were Windows (NTFS, a local SSD), in an isolated export, with other
agents' builds running on the machine; Linux ran the unit and integration tests only.

## Method

- Engine admission: `copy_operations::tests::streamed::measure_admission` (release
  build, `--ignored --nocapture`): a Job of 1,000,000 existing files of 16 bytes in one
  folder prepares, takes and copies its first item, then reads every remaining identity;
  it reports the times and the memory the selection keeps beside its paths (capacities of
  its tables, `streamed_retained_for_test`).
- Whole application path: `measure_transfer` (release build) enqueues a Copy, a Move on
  the same volume, or a Move to another volume (`MIDNIGHT_STREAMED_MOVE_TO`) of the files
  through the copy actor, admission and namespace claim, and reports the time to the first
  completed item and the total. Peak working set and handle count of the test process
  were sampled every 100 ms by PowerShell (`Get-Process`).

## Results

### Engine admission, 1,000,000 files in one folder (release)

| Measure | Value |
|---|---|
| Job prepared (`prepare_streamed`, discovery started) | 0.33 ms |
| First item copied, from the start of preparation | 24 ms |
| Every identity read (`identify_streamed_for_test`) | 190 s, 190 µs per item |
| Kept by the selection | 194 MB: 83 MB of paths, 111 bytes per item beside them |
| Test process, peak working set / most handles | 254 MB / 142 |

The identity pass runs inside the job's steps, 2 ms after each item the job takes, and a
conflict reads the remaining identities first (in 5 ms slices, so pause and Cancel stay
prompt). Its cost is one open of each item, `prepare_transfer_source` (190 µs on this
machine), plus one more open when the job takes the item.

### Through the copy actor, 20,000 files of 16 bytes (release)

| Selection | First file done | Total | Rate | Peak working set | Most handles |
|---|---|---|---|---|---|
| 20,000 items (streamed) | 0.26 s | 91.3 s | 219 files/s | 31 MB | 159 |
| their folder, one item (prepared) | 0.25 s | 84.4 s | 237 files/s | — | — |

The streamed selection costs about 8 % against copying the folder that holds the same
files, which the identity pass and the second open per item explain. An earlier run of the
1,000,000-file Copy while a debug build of the application compiled beside it copied
75 files/s with the pass only one item ahead: each item was walked by `discover`, about
2 ms under that load. A file's totals now come from the entry read as it is opened
(`copy_streamed_roots.rs`), and the 20,000-file runs above are with that change.

## Per-item memory

The unit test `a_streamed_selection_keeps_about_a_hundred_bytes_per_item_beside_its_paths`
measures 106 bytes per item beside the paths for 20,000 items (debug build); the paths
themselves take the selected path's length plus 24 bytes each, shared with the request.
