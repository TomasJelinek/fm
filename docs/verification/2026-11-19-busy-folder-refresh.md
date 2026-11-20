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

## Follow-up: refreshes that change nothing visible (20 November 2026)

A folder that changes all the time, such as `/tmp`, still blinked about once a
second after the change above, although the rows were edited in place. Three
row decorations were dropped by every background re-read and rebuilt afterwards:

- **Folder presence** (the expander arrow of folder rows). Dispatching a same-folder
  re-read cleared the presence cache (`clear_tree_presentation`), and while the
  re-read was pending `root_disclosure_available()` was false, so every folder row
  lost its arrow; after the commit the new navigation and index no longer matched
  the cache, so the arrows came back as unknown until probed again.
- **Filename colors** were matched to the exact rows `Arc`; a commit that edited any
  row dropped every colored name's style until the backend matched the new rows.
- **Thumbnail identities** contained the committed navigation, so thumbnails view
  requested every visible thumbnail again after each commit.

Now a background re-read of plain rows keeps the presence cache and shows it while
pending (`Pane::root_disclosure_shown`; actions still use
`root_disclosure_available`), and an in-place commit carries the hints of folders
whose path, kind and modification time are unchanged over to the new rows
(`carry_folder_presence`); new or changed folders are probed. Filename styles of
rows an in-place commit edited stay in use until the match of the new rows
arrives. Thumbnail identities use the held row generation plus the file's length
and modification time.

Slint repaints a row only when its data differs, so identical rows are not redrawn
by the presentation pass's row notifications. One caveat: Slint 1.18 never finds
two empty images equal (`ImageInner::None` falls through to `false`), so a row whose
icon is not loaded yet is re-dirtied by each pass; it shows the same pixels.

New tests (`busy_folder_native`, real Engine and inotify, the rows' viewport reported
as the application's visible-row pass does):
`a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched`
(no row model reset and every row identical in every frame across a re-read, with a
selection, a colored file and resolved folder arrows) and
`a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints` (only the
grown file's row changes, in place with its identity).

## Review fixes (20 November 2026)

An independent review of this change and of the post-operation refresh found:

- **A row moving up refilled the panel** (`listing_refresh::row_edits`). When the
  next shown and listed rows differed and both were on either side, the shown row
  was always taken as the one that moved. A row that jumped k rows up cost about 2k
  edits and, past the 256-edit limit, replaced every row (new identities, colors
  and folder hints dropped, thumbnails requested again): in a folder sorted newest
  first, every rewrite of an older file did that. Both sides' row positions are now
  indexed and the row that moved further is the one removed and added again, so a
  move costs two edits in either direction.
- **Background re-reads refused pane actions silently.** While a re-read kept the
  rows on screen, Rename, Delete, New file, New folder and expander clicks did
  nothing (Delete reported that the list was updating), although nothing showed a
  loading state. They now wait for the re-read and run once it commits or fails, on
  the targets captured when they were requested, unless the panel left the folder
  (`run_deferred_actions`). The Engine prepares an operation only against the
  listing the UI acknowledged, so a waiting operation acknowledges the committed
  listing first. Opening an entry uses the rows on screen at once.
- **Folder hints carried over too eagerly** (`carry_folder_presence`). Hints were
  carried when a folder's path, kind and modification time were unchanged. A
  directory link's listing metadata describes the link, not its target, a chmod
  changes no modification time, and an unavailable folder can become readable
  without a visible change; those hints are now probed again (permissions and
  attributes are compared too).
- **Tests.** The "row model was replaced" check compared a `ModelRc` that is set
  once at startup, so it could not fail; a model listener now counts resets. The
  "changes nothing visible" test created and removed a file, which a re-read
  starting in between lists; it now sets a file's modification time to its own
  value twice.

The panel input hold after a file job changed too (per panel, keys kept and
replayed, only for jobs the user waited for); see
[Operation progress](../developer/OPERATION_PROGRESS.md). The tree worker now sends
a refresh pass's reply only after every view of the pass, so a tree's hold cannot
end before the view that shows a renamed entry.

New tests: `listing_refresh::tests::a_row_that_jumps_far_up_or_down_costs_two_edits`
and `tree_controller::worker::refresh_tests::refreshed_never_overtakes_a_publication_of_its_pass_parked_behind_a_commit`
(engine); eleven presenter tests in `presenter::tests::{operations, tree::rename,
folder_presence}` (per-panel holds, a Copy's sources, long jobs, deferred actions,
opening during a re-read, kept reveals, a tree refresh that cannot start, carried
hints); `panel_input_hold` (five UI tests: replay order and modifiers, the other
panel and the command line, focus moved away, the 3 s expiry on a virtual clock,
a native-hook shortcut); and
`busy_folder_native::a_file_that_jumps_to_the_top_moves_without_refilling_the_rows_it_passed`
(600 files sorted newest first, real Engine and inotify). The native F2 test now
checks that an Up typed during the hold is replayed from the revealed new name.

| Suite (Linux, debug, project Docker image) | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-engine` library, filters `listing` and `tree_controller` | 88 | 0 | 1 |
| `midnight-ui` library | 1283 | 0 | 2 |
| `midnight-ui` integration, filters `panel_input_hold`, `mutation_controls`, `busy_folder`, `operations_native_ui`, `tree`, `rename` | 233 | 0 | 1 |

Clippy (`--all-targets -- -D warnings`) is clean for `midnight-engine` and
`midnight-ui`. Windows was not run; the native and busy-folder tests used inotify.
In one of five runs of the integration set, on a host busy with other builds,
`a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges` failed in
its final convergence wait (`converge_silently`; the message was not captured). It
passed in the other four runs of the set and in three runs on its own.

## Follow-up, 20 November 2026

**Watch registration failures.** A panel showing `/tmp` said that change
notifications were unavailable and the folder was checked about every 15 seconds;
its tooltip read `OS file watch limit reached. about ["/tmp"]` (inotify's per-user
limit, with the debug path list of the `notify` error). Now every subscription of
the Linux filesystem service shares one inotify instance: a directory watched by
several subscriptions is watched once, its native watch ends with its last
subscription, and a directory replaced since its watch was added is watched anew
without the old directory's watch (whose later removal would otherwise end the new
one inside `notify`). A flat folder whose registration fails is checked about every
second (less often above 4,000 entries, never more than a tenth of the time) while
the registration is retried after 1, 2, 4 and 8 seconds and then every 15 seconds;
a retry that succeeds switches to notifications without a status, and only
registrations that keep failing for a minute show the line, which now reads "Live
updates are unavailable for this folder; it is checked for changes regularly
instead." with the reason (such as the `fs.inotify.max_user_watches` limit) in
plain words. Tree views are unchanged: they report at once and register again with
a later scope update. Windows already shares one watch service and was not run.

**Hold during slow jobs.** After a Rename the wait cursor could end before the new
name appeared, because the 3 s limit counted from the confirmation. A confirmed
Rename, New file or New folder now holds its panel for as long as the job runs,
and its status line names the job (`Renaming “a” to “b”…`) after half a second;
the 3 s limit applies only to the re-read after the completion, after which input
is released and the line says `Updating the file list…` until the change appears.
A job the panel waited for counts as waited for however long it ran, so its
re-read still holds the panel. Escape ends either wait (the keys typed before it
are replayed), and a failed or cancelled job releases the panel at once.

New tests: `watch::shared_tests` (six: one instance with per-directory counts, a
replaced directory, a watcher that cannot start, a failed directory, dispatch,
plain limit errors) in `midnight-platform-linux`; three engine watch tests (a
silent successful retry, frequent checks with a late status, the check interval)
and `watch_native::a_failed_registration_is_checked_often_and_retried_without_a_status`;
`presenter::tests::operations::a_rename_holds_its_panel_while_it_runs_however_long_and_then_for_its_reread`;
and in `panel_input_hold` a 5 s job on a virtual clock, the re-read limit counted
from the completion, Escape and a failed job.

| Suite (Linux, debug, project Docker image) | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-platform-linux` library, filter `watch` | 14 | 0 | 0 |
| `midnight-engine` library, filters `watch`, `listing_refresh` and `tree_controller` | 122 | 0 | 0 |
| `midnight-engine` integration, filter `watch` | 12 | 0 | 0 |
| `midnight-ui` library | 1284 | 0 | 2 |
| `midnight-ui` integration, filters `panel_input_hold`, `mutation_controls`, `operations_native_ui`, `busy_folder_native`, `tree_rename`, `tree_watch`, `thumbnail_grid_rows`, `panel_warning_details`, `keyboard_options`, `interaction_controls`, `commander_edges`, `function_bar`, `status_bar`, `native_operation_windows` | 144 | 0 | 1 |

Clippy (`--all-targets -- -D warnings`) is clean for `midnight-platform-linux`,
`midnight-engine` and `midnight-ui`. The whole `midnight-platform-linux` library
in the same image passed 333 tests and failed 37 transfer-namespace fixtures that
stop with "fixture requires qualified local byte lookup"; they do not use the
watch code, and the image is not qualified for them
([test execution](../developer/TEST_EXECUTION.md)).

## Remaining risks and follow-ups

- Watch callbacks carry no names, so every refresh still re-reads the folder.
  Per-name updates would need a `WatchService` change on Linux, Windows and the
  native filesystem plugin host.
- After each accepted re-read of a busy folder the watch target probes it at once
  (a second full read per cycle) before the next hint.
- Pane actions outside the panel and operation modules (viewers, Custom Commands,
  inserting a name into the command line, the archive Copy dialog) still check the
  pane's internal loading state and do nothing during a background re-read.
- Thumbnail identities depend on a file's length and modification time; listings
  carry no change stamp, so a file rewritten with the same length and modification
  time (`cp -p`, `rsync -t`, a 2 s FAT timestamp) keeps its old thumbnail until the
  folder is opened or refreshed explicitly.
- A flat folder whose registration never succeeds (a full watch limit that stays
  full, or a filesystem that refuses notifications) keeps being checked about
  every second, bounded only by the tenth-of-the-time rule; on a slow network
  share that is still more traffic than the former 15 s check.
- Tree views still report a failed registration at once and are checked at the
  regular interval; they do not use the retry and frequent checks of flat folders.
- `notify` drops a watch whose directory is moved or deleted together with every
  watched directory below it, also those of other subscriptions in the shared
  instance; those subscriptions get no event for it and rely on their regular
  check until a refresh registers them again, as before when their directory was
  moved with its parent.
- The hold phases were tested with the Slint testing backend and a virtual clock;
  the wait cursor and status line were not checked in a desktop session.
