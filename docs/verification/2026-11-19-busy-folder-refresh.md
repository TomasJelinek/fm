# Busy folder refresh — 19 November 2026

Problem: a panel showing a large folder whose files keep changing (a cargo build
writing `_build/debug/deps`, about 5,800 entries) kept refreshing. Each refresh
showed the progress bar, re-read and re-filled the whole panel, and the next one
started as soon as it finished.

## Cause

Every debounced watch hint (`Event::DirectoryChanged`) made the presenter send a
`Command::Refresh` as an ordinary pending navigation:

- The pane went into its loading state (`Pane::loading`), which the panel shows
  as the progress bar once a listing outlives the feedback delay, and which also
  holds back pane actions until the listing commits.
- The Engine streamed the whole folder as `EntriesBatch` and `MetadataBatch`
  events (about 92 batches of 128 rows for 5,800 entries) through the bounded UI
  queue, although the presenter discards them for a same-folder re-read.
- Every entry got a new `EntryId`, sort keys and localized time, and the row
  identities included the pending and committed generations. The row model was
  reset twice per refresh (at dispatch and at commit) and every visible row was
  replaced.
- Nothing limited consecutive hints: when a listing of a busy folder committed,
  the watch target had counted changes during it, probed the folder at once and
  hinted again after the 150 ms / 500 ms debounce. Under build load each cycle
  took longer, so the panel was loading most of the time.

## Fix

- **Engine, in-place re-read** (`crates/midnight-engine/src/listing_refresh.rs`).
  A `Refresh` of the folder whose rows the UI acknowledged (`PaneCommitted`)
  streams no row batches. Each entry of a shown path and kind keeps the shown
  entry's identity, sort keys and localized time (only changed metadata is
  localized again); an identity that would collide with a new entry's restores
  the identities the listing read. After sorting, the listing is described as
  row edits of the shown rows (`Event::ListingChanges`, sent just before
  `ListingFinished`; at most 256 edited rows, otherwise omitted). If nothing
  changed, `ListingFinished` carries the very same rows.
- **Watch rate limit** (`crates/midnight-engine/src/watch.rs`). A flat target
  hints a folder that keeps changing at most once per `Timing::cooldown` (1 s, or
  twice its last check's duration up to 4 s), measured from the previous hint, so
  the limit spans the refresh it caused. An isolated change is still hinted after
  the 150 ms quiet period.
- **UI, silent and in place** (`crates/midnight-ui/src/presenter/row_continuity.rs`,
  `crates/midnight-ui/src/lib.rs`). A background re-read (`Reload::Background`)
  shows no loading state (`Pane::loading_shown`); the pane still waits for it
  internally as before. While it is pending the rows keep their identities, and
  when it commits a plain flat view (no tree, search filter, archive or network
  view) whose rows are the edits' base, the row model applies the edits as row
  removals, insertions and changes instead of a reset. Cursor, selection and
  viewport follow the existing same-folder commit rules. Any other listing, or a
  burst beyond the edit limit, replaces the rows as before, still without the
  loading state for a background re-read.

## Measurements

Benchmark `busy_folder_native::busy_folder_benchmark` (ignored by default):
the real UI, presenter, Slint software window, production Engine, locale service
and inotify watcher over a folder of 5,800 files plus fixtures, while a writer
creates, renames, appends to and removes object files every 2 ms for 8 s. Linux,
debug build, inside the project Docker image, on a host shared with other
sessions' builds (so absolute times vary). Before = `ec96710a` plus only the
benchmark files; after = this change.

```
cargo test -p midnight-ui --test integration busy_folder_benchmark -- --ignored --nocapture
```

| Metric (8 s of changes) | Before, run 1 | Before, run 2 | After, run 1 | After, run 2 |
| --- | ---: | ---: | ---: | ---: |
| Frames showing the loading state | 7.4% | 31.7% | 0% | 0% |
| Full listings | 8 | 9 | 8 | 7 |
| Mean listing time (start to commit) | 173 ms | 328 ms | 194 ms | 376 ms |
| Row batches streamed to the UI | 736 | 828 | 0 | 0 |
| Times the cursor row was replaced | 16 | 17 | 0 | 0 |
| Converged after the writer stopped | 0.58 s | 0.72 s | 1.02 s | 1.07 s |
| Isolated new file shown after (idle folder) | 0.46 s | 0.46 s | 0.49 s | 0.52 s |
| Loading state for that isolated file | yes | yes | no | no |

The backend re-read still reads every entry (`statx` and the platform's localized
time per entry on Linux), so its duration is unchanged; it no longer reaches the
screen as a progress bar, a refill or a stream of batches, and pane rows keep
their delegates. Convergence after a busy period now waits for the 1 s cooldown.

## Tests

New:

- `listing_refresh::tests` (7, engine): edit scripts reproduce the new rows for
  additions, removals, changes, moves, recycled identities and an empty side; a
  reversed listing is replaced; adoption keeps identities and localized times
  and restores read identities on a collision.
- `watch::tests::a_folder_that_keeps_changing_is_hinted_at_most_once_per_cooldown`
  (engine): an isolated change is hinted before the cooldown; a writer that never
  pauses is hinted at least a cooldown apart, and its last changes are hinted.
- `busy_folder_native` (UI integration, real Engine and inotify):
  `small_change_bursts_update_rows_in_place_without_the_loading_state` (no loading
  state, no row batches, cursor row identity, cursor and selection kept, grown file
  shown), `a_large_burst_replaces_the_rows_and_still_converges_without_the_loading_state`
  (400 new and 150 removed files) and
  `a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges`.

Results (debug, Linux, project Docker image; all workspace sources dated ahead
of the shared target directory's artifacts so no other worktree's build was linked):

| Suite (after rebasing on `513d817b`) | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-ui` library | 1253 | 0 | 2 |
| `midnight-ui` integration (includes `busy_folder_native`) | 788 | 0 | 2 |
| `midnight-engine` integration | 417 | 0 | 7 |
| `midnight-engine` library, `listing_refresh` and `watch::` | 91 | 0 | 0 |
| `midnight-engine` library, all (before the rebase) | 1678 | 19 | 16 |

The 19 engine library failures are all in `copy_operations` suites that this change
does not touch: 8 fixtures refuse the container's bind-mounted temporary directory
("requires a supported local lookup domain") and 11 time out waiting for native
copies in the same environment. They were not re-run on the base commit.
Clippy (`-D warnings`) is clean for `midnight-core` and `midnight-engine` with all
targets, and for the `midnight-ui` library and integration targets; the
`midnight-ui` library test target stops on two existing `cloned_ref_to_slice_refs`
findings in `about_licenses.rs`.

## Remaining risks and follow-ups

- Watch callbacks carry no names, so every refresh still re-reads the folder.
  Per-name updates would need a `WatchService` change on Linux, Windows and the
  native filesystem plugin host.
- After each accepted re-read of a busy folder the watch target probes it at once
  (a second full read per cycle) before the next hint.
- The pane's internal loading state still holds back pane actions for the
  duration of a background re-read, as before.
- Thumbnail identities still use the committed generation, so thumbnails view
  requests the visible thumbnails again after each background commit.
