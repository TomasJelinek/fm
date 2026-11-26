# Drive list follows drive changes — 26 November 2026

The owner reported that the drive bar and the drive dropdown noticed an inserted or
removed USB stick only after a long delay. The drive list now follows the system's
notifications ([when drives come and go](../DRIVE_INFORMATION.md#when-drives-come-and-go)).

## Cause

Nothing told the engine that a drive had come or gone. The window asked for the drive
list at startup, after a network connection and after opening a Network share; besides
that, `volumes::Controller` listed the drives again only every 45 seconds. A new or
removed drive therefore showed after 0 to 45 seconds, 22.5 seconds on average. Listing
itself was never the slow part: on Windows it is one `GetLogicalDrives` call, on Linux
one read of the mount table, both on a backend worker, and labels and free space were
already read afterwards. A panel on a removed `subst` or network letter also waited for
its folder's 15-second check before it moved away.

## Change

- `WatchService::watch_volumes` (midnight-core): a hint that the drive set may have
  changed. Unsupported by default.
- Windows (`midnight-platform-windows/src/volume_watch.rs`): one thread with a hidden
  top-level window (message-only windows receive no broadcasts) gets `WM_DEVICECHANGE`
  `DBT_DEVICEARRIVAL`/`DBT_DEVICEREMOVECOMPLETE` for `DBT_DEVTYP_VOLUME`, the shell's
  `SHCNE_DRIVEADD`, `SHCNE_DRIVEREMOVED`, `SHCNE_MEDIAINSERTED` and `SHCNE_MEDIAREMOVED`,
  and compares `GetLogicalDrives` every 250 ms, hinting only when the mask changed. The
  broadcast for `subst` arrived 45 to 620 ms after the command (it is delivered to every
  top-level window in turn), so the mask comparison bounds the delay. No new crate or
  `windows-sys` feature.
- Linux (`midnight-platform-linux/src/volume_watch.rs`): one thread blocks in `poll` for
  `POLLPRI | POLLERR` on `/proc/self/mountinfo` and an eventfd that stops it. GVfs
  locations, folders of one FUSE mount, raise no notice and keep the existing behavior.
- Engine (`volumes.rs`): the watch is registered on a blocking worker at the first
  `ListVolumes`; a hint marks a watch slot, so any number of hints cause at most one
  listing in flight and one owed. A listing started by a hint or the 45-second timer is
  published only when it differs from the last published list; an explicit request is
  always answered. A hint also drops cached details whose read failed (an empty card
  reader that got a card). New drives are published before their details are read.
  The subscription is dropped on a blocking worker when the actor ends.
- Presenter (`removed_volumes.rs`): a panel whose folder is on a volume that left the
  list (the longest listed root containing its path) re-reads its folder at once, as a
  watch hint would, so the existing recovery (nearest parent that opens, then the
  system drive or `/`) runs immediately.

## Measurements (Windows 11, debug build)

`subst <letter>: <own temporary folder>` and `subst <letter>: /d`, the letter removed in
a drop guard. Engine latency is measured from the `subst` command's exit to the engine's
`Event::VolumesUpdated` with or without the letter, with the real `WindowsFileSystem`.

| Build | Added | Removed |
| --- | --- | --- |
| Before (no drive watch, `without_a_drive_watch_only_the_periodic_refresh_lists_a_subst_drive`) | 44.77 s | 44.94 s |
| After (`a_subst_drive_is_listed_and_unlisted_within_half_a_second`, 5 runs) | 9–19 ms | 3–58 ms |

The "before" figure is the timer's phase in that run; any value from 0 to 45 s is
possible. The platform test, which records the hint's source, saw the system broadcast
45 to 620 ms after the command started with a 1-second mask comparison; with 250 ms the
comparison usually reports the change first.

## Tests

| Command | Result |
| --- | --- |
| `cargo test -p midnight-platform-windows --lib watch -- --include-ignored` | 18 passed (with the 250 ms comparison: `subst` hinted 18 ms and 34 ms after the command started) |
| `cargo test -p midnight-engine --lib volumes::` | 3 passed |
| `cargo test -p midnight-engine --test integration volumes` | 8 passed, 2 ignored (the `subst` tests, run separately with `--ignored`: both passed) |
| `cargo test -p midnight-core --lib watch` | 2 passed |
| `cargo test -p midnight-ui --lib presenter::tests::volumes` | 6 passed |
| `cargo check -p midnight-platform-linux --tests --target x86_64-unknown-linux-gnu` | compiled on Windows; the Linux tests did not run here |

New tests: the Windows message filter and window lifetime; the `subst` latency (ignored,
it changes the session's drive letters); engine hints with a fake watch (a new drive is
listed within 200 ms of the hint while both details workers are blocked, a removal, an
unchanged list publishes nothing, 500 hints cause one or two listings, failed details
are read again); inventory comparison, request priority and failure expiry; the Linux
`poll` result classification and a watch that starts, sends nothing and stops; the
presenter's re-read of only the panels on a removed volume, also with nested mounts.

## Not verified

- A real USB stick or card inserted and removed (the owner's machine was not to be
  touched); `subst` uses the same `DBT_DEVTYP_VOLUME` broadcast.
- Linux: the mount watch was compiled, not run; no mount was made.
- The running application window (only the engine and presenter were exercised).
