# Deletion latency on Linux — 25 November 2026

Problem (user report): on Linux, after files are deleted, the free disk space in
the panels, and sometimes the panel itself, takes a second or more to update.

## Measurement

`delete_latency_native::delete_latency_benchmark` (UI integration test, ignored by
default) drives the real UI, presenter, production Engine and inotify watcher, as
`busy_folder_native` does. The left panel shows the folder of a 2 GiB file (written
and synced), the right panel another folder on the same volume. Each scenario
deletes the file and records when the left panel's row disappears and when each
panel's free space shows at least half of the freed space, with every watch hint,
listing and operation event. Linux, debug build, project Docker image, ext4 on
LUKS + LVM on a SATA SSD, on a host that other sessions' builds kept busy.

```
cargo test -p midnight-ui --test integration delete_latency_benchmark -- --ignored --nocapture --test-threads 1
```

Milliseconds from the deletion (for the application's Delete, from Enter in its
confirmation) until the left row disappeared / the left panel's free space / the
right panel's free space showed it, two rounds each. Another program's deletion
runs on a thread of its own, as `rm` in a terminal does. "Before" is the base
commit `2ca28d70` with only the benchmark added; "after" is this change; both used
the same benchmark.

| Scenario | Before | After |
| --- | --- | --- |
| `rm` in the shown folder, idle | 2,701/2,701/5,914; 321/321/1,375 | 260/260/260; 241/241/241 |
| `rm` 300 ms after another change of the folder | 954/954/954; 984/984/1,623 | 268/268/324; 251/251/312 |
| Application Delete (Shift+F8, Enter) | 300/300/1,114; 296/296/1,085 | 209/209/264; 208/208/263 |
| `rm` in a folder no panel shows (free space only) | 2,135/2,135; 818/1,493 | 2,502/2,502; 524/524 |

Each scenario's event timeline separates the kernel's work from the application's.
Measured from the moment the deletion could be seen (`unlink` returned; the
Delete's terminal progress arrived):

- `rm` 300 ms after another change: before, the row followed 488 and 870 ms later
  (the watch hint itself was late), against 147 and 201 ms for `rm` in an idle
  folder; after, 174 and 128 ms, and the right panel's free space 230 and 189 ms.
- `rm` in an idle folder: before, the right panel's free space followed 3,360 and
  1,255 ms later (its periodic read); after, in the frame of the left panel's
  update, 166 and 146 ms.
- Application Delete: before, the right panel's free space followed 814 and 789 ms
  after the operation completed (the throttle); after, 55 ms. The left panel showed
  both at completion in either version.
- A folder no panel shows is still read only periodically (every 5 seconds).

The first "before" round shows how long the kernel itself can take: `unlink` of
the synced 2 GiB file returned only after 2,554 ms while other builds kept the disk
busy. In runs of earlier versions of this change it took 4 to 6 seconds once each;
the row disappeared as soon as the watch reported the removal (for a Delete, with
the operation's completion). Space released later than the last follow-up read (3
seconds after the request) shows with the next periodic read.

An earlier run of the base code, whose benchmark still removed the file on the UI
loop's thread, gave the same picture: watch hints after 1,029 and 2,268 ms for
`rm` 300 ms after another change (237-256 ms in an idle folder), and the right
panel's free space 1,074-1,075 ms after a Delete that completed at 190-197 ms.

## Causes

1. **Watch hint cooldown** (`crates/midnight-engine/src/watch.rs`,
   `Target::hint_deadline`, `Timing::cooldown` = 1 s, stretched to twice the
   folder's last check, up to 4 s). It was measured from the previous hint and
   applied to every burst: a change shortly after an earlier one waited for it, and
   so did the last changes of a long deletion by another program. A check made
   while the disk is busy (as during and after writing or deleting large files)
   can take seconds even in a small folder, which stretched the wait to several
   seconds.
2. **Free-space request throttle** (`crates/midnight-engine/src/free_space.rs`,
   `Timing::throttle` = 1 s after the previous read, for every storage kind). A
   Delete asks for the free space of each panel on its volume with its Running
   progress and again with its terminal one; the second request waited a second
   after the read the first one caused. The deleting panel's own re-read showed
   the new value at once; the other panel on the volume did not.
3. **No free-space request after a watched change.** A watch hint re-read the
   changed folder, whose listing reads its free space, but the other panel on the
   same volume waited for its periodic 5-second read.
4. **ext4 reports a deleted file's space after `unlink` returned** (filesystem
   behaviour, not an application timer). `statvfs` polled every 0.5 ms after
   `unlink` on the same volume (Python): a 256 MiB / 1 GiB / 2 GiB file was
   reported freed 12.6 / 50.2 / 96.7 ms after an `unlink` that returned in
   0.1 ms; `statvfs` right after `unlink` of a 512 MiB file still reported 0 MiB
   freed. Another `unlink` of a synced 2 GiB file took 1,360 ms while the disk was
   busy. A single read right after a deletion can therefore show the old value,
   which then stayed until the next read (a second or the periodic five).

Windows uses the same engine workers, so causes 1-3 apply there too; its watch
service and volume reads add no timers of their own. Cause 4 is Linux (ext4)
behaviour. The Linux watch adapter (`midnight-platform-linux/src/watch.rs`, the
`notify` 8.2.1 inotify backend, which polls without a timeout) delivers each event
at once and is unchanged.

## Fix

- **Watch** (`watch.rs`, `hint_deadline`): a burst that settled (150 ms without a
  change) is hinted at once, however soon it follows the previous hint: a change
  after a quiet moment, and the last changes of a folder that kept changing. A
  burst that goes on is still hinted once it lasted 500 ms and no sooner than the
  cooldown (1 s, up to 4 s for a folder whose check takes long) after the previous
  hint, so a folder that keeps changing is still listed at most once a second and a
  large deletion still refreshes once per second, not once per file.
- **Free space, engine** (`free_space.rs`): requests on a local disk (`LocalSsd`,
  `Rotational`, `Local`) are spaced by 200 ms instead of a second (network and
  unclassified storage keep a second). While the window is visible, a request
  keeps a local disk followed for four seconds: its free space is read again 0.2,
  0.6, 1.4 and 3 seconds after the request, and a value is sent only when it
  changed. A later request starts the sequence again, so a folder whose watch hints
  keep coming costs a few reads a second.
- **Free space, presenter** (`presenter/free_space.rs`,
  `Presenter::folder_changed_space`): a watch hint's re-read and a tree refresh
  ask for the free space of every panel on the folder's volume: the others at
  once, the re-reading panel once its listing commits.

## Tests

New tests. Before the fix, the first five failed on the base code as noted (the
follow-up test then released the space 600 ms after the request; it now does so
after 1.2 s, between the final follow-up reads):

- `free_space::tests::a_local_disk_answers_a_request_soon_after_its_previous_read`
  (engine, paused clock; base: answered after 1 s).
- `free_space::tests::a_request_follows_a_local_disk_until_a_deletion_released_its_space`
  (engine, paused clock; base: the request was not read within 600 ms).
- `watch::tests::a_change_after_a_quiet_moment_is_hinted_once_it_settles_despite_the_cooldown`
  (engine; base: hinted 899 ms after the change).
- `watch::tests::the_last_changes_of_a_folder_that_kept_changing_are_hinted_once_they_settle`
  (engine; base: hinted 825 ms after the last change).
- `presenter::tests::free_space::a_watched_change_asks_for_the_free_space_of_every_panel_on_its_volume`
  (UI; base: no request for the other panel).
- `watch::tests::timing_tests::production_hints_a_change_after_a_quiet_moment_once_it_settles`
  and `production_hints_a_folder_that_keeps_changing_once_a_second_and_its_last_changes_at_once`
  (engine, explicit instants with the production timing).

Changed: `watch::tests::a_folder_that_keeps_changing_is_hinted_at_most_once_per_cooldown`
no longer requires the hint of the last changes to wait for the cooldown, and checks
the spacing only when the writer never paused for the settling time (a stalled
writer makes a quiet moment).

Results (Linux, debug, project Docker image, private target directory):

| Run | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-engine` library, `free_space::` and `watch::` | 102 | 0 | 0 |
| `midnight-engine` integration, `watch_native` (inotify) | 14 | 0 | 0 |
| `midnight-ui` library (all) | 1,729 | 0 | 11 |
| `midnight-ui` integration: `busy_folder_native`, `operations_native_ui`, `tree_watch`, `focus`, `native_operation_windows`, `every_test_file_is_a_module` | 55 | 0 | 1 |

`cargo fmt --all -- --check` is clean. `cargo clippy -p midnight-engine -p midnight-ui
--lib --tests -- -D warnings` stops on a finding the base commit already has
(`clippy::len_zero` at `crates/midnight-engine/src/operation_delete.rs:246`, from
`4e3e64c6`); with only that lint allowed (`-A clippy::len_zero`) it is clean.
