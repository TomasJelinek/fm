# Linux drive names on the drive toolbar, 25 November 2026

On Linux the drive bar and the drive dropdown showed each drive by its whole mount
path, for example `/media/<user>/<label>`. They now show a short name, and the tooltip
gives the whole path ([drive names](../DRIVE_INFORMATION.md#drive-names)). Windows keeps
its drive letters.

## Change

- Both Linux adapters name each drive as they list it: the native filesystem package,
  whose list the application shows (`plugins/midnight-filesystem-native-linux`), and
  the application's own adapter, which the acceptance harness and the desktop
  locations use (`crates/midnight-platform-linux`). Each has its own copy of
  `drive_names.rs`, because the package depends on no host crate.
- `/` stays `/` and Home stays Home. A filesystem with a udev label shows it, any other
  mount its own folder. The label is read from udev's record of the device number in
  `/proc/self/mountinfo`, once per device, never from the mounted filesystem.
- The locations opened through GVfs, which the package lists by name since the
  [GVfs drive list](2026-11-25-gvfs-drive-list.md) change (`ee15fc9f`, which also left
  the bridge out), keep those names as their labels and follow the same rules.
- A name over 16 columns keeps 8 columns before `…` and 7 after it. Names that would be
  equal take folders from the end of their paths until all differ; only then the whole
  path, without its leading `/`.
- The engine lists `/` first; before, a mount-path label sorted after every name.
- The UI shows the name on the bar and in the dropdown. The tooltip gives the name, the
  whole path and the free space. The bar button's accessible name is that tooltip, and a
  dropdown row's accessible description is its tooltip. A label that its root's text
  starts with (a Windows drive letter, `/`) keeps the former presentation.
- The accessible description comes from the shared picker: a row of any picker whose
  tooltip differs from its label now describes itself with that tooltip, on Windows
  too. Nothing visible changes there.

## Tests

All builds ran in the project's Docker build image with a private `CARGO_TARGET_DIR`
under the worktree's `_build/`.

| Command | Result |
| --- | --- |
| `cargo test -p midnight-platform-linux --lib -- drive_names mounts:: volume_information` | 30 passed: 10 name rules, 16 drive-list tests (2 new: names from udev labels and mount folders, one label read per device, the top mount of a path, hex identities), 4 volume information tests |
| `cargo test --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml` | 96 unit tests (the same 10 name rules and 2 drive-list tests, the GVfs tests with the location names now shortened like any name) and 26 integration tests passed |
| `cargo test -p midnight-engine --lib --test integration -- navigation volumes` | 65 and 43 passed; `linux_drives_list_the_filesystem_root_first_then_their_names` is new |
| `cargo test -p midnight-ui --lib --test integration` | 1731 and 1241 passed (11 ignored each) before the GVfs change was merged; after it, the drive, glyph and presentation tests again (the merge changed only the UI test's sample drive) |
| `cargo test -p midnight-platform --tests` | 17 passed |
| `cargo clippy --features midnight-acceptance/acceptance -p midnight-platform-linux -p midnight-engine -p midnight-ui -p midnight-acceptance --all-targets -- -D warnings` | clean |
| `cargo clippy --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml --all-targets -- -D warnings`, `cargo fmt --check` | clean |

`listed_volumes_show_names_of_their_own_and_keep_whole_roots` (package integration) asks
the provider for its volumes, also with tmpfs mounts at `/mnt/USB` and
`/media/alice/USB` (`mnt/USB`, `alice/USB`). The new UI tests are
`drive_names::linux_drives_show_short_names_with_the_whole_mount_path_in_tooltips`
(captions, tooltips, menu rows without the path, the hover tooltip, the rows' accessible
descriptions) and `drive_names::drive_letters_keep_their_root_in_the_menu_and_tooltip`.

Each new test of changed behaviour failed first: the name rules against a stub that
returned whole paths, the drive-list tests against the former whole-path labels, the
package's volume-list test with the former whole-path names (with the two tmpfs mounts),
the ordering tests with `/` last, and the Linux UI test on the former tooltips and, with
the row's description removed again, on the description (`USB — 1.0 GiB free: ""`).
`drive_letters_keep_their_root_in_the_menu_and_tooltip` passed before and after: it
guards the unchanged Windows presentation.

The whole `midnight-platform-linux` lib suite fails 37 `transfer::namespace` tests in
the image with "fixture requires qualified local byte lookup": the image's temporary
folder lies on the bind-mounted `_build`, which those fixtures cannot qualify (32 of
them pass with a tmpfs temporary folder). Run natively on the workstation, with an
ext4 temporary folder and the same toolchain, the crate's lib suite passed (496 passed,
36 ignored) and its integration tests too (42 passed).

## Acceptance

The new scenario `panels_drive_names` ran three times on the merged code with
`--only panels_drive_names`; `panels_paths`, whose drive-menu checks look for `/`, passed
with the drive fixture too.

| Run | Drives | Result |
| --- | --- | --- |
| `20261125-213455`, Docker image with the drive fixture ([ACCEPTANCE.md](../developer/ACCEPTANCE.md#drive-names)) | `/`, `a-very-l…er-name`, `alice/USB`, `backup/data`, `mnt/USB`, `srv/data` | PASS, 3 screenshots |
| `20261125-213516`, Docker image without it | `/` | PASS, 3 screenshots; "no drive fixture mounted" |
| `20261125-213518`, the workstation (Ubuntu 26.04, a GNOME session) | `/`; the application's own adapter leaves the GVfs bridge out | PASS, 3 screenshots |

An earlier workstation run, before the GVfs change was merged, listed the session's
GVfs bridge as `GVfs` and its tooltip gave the bridge's whole path.

The screenshots of `20261125-213455`, inspected:

- `drive-bar.png`: the bar reads `/`, `a-very-l…er-name`, `alice/USB`, `backup/data`,
  `mnt/USB`, `srv/data`, then Network.
- `drive-tooltip.png`: hovering the first drive other than `/` in the mount table's
  order (`mnt/USB` in this run) shows "mnt/USB", "/mnt/USB" and "62.8 GiB free".
- `drive-menu.png`: Alt+F1 lists "/ — 88.3 GiB free", "a-very-l…er-name — 62.8 GiB
  free", "alice/USB — 62.8 GiB free" and so on to "srv/data — 62.8 GiB free", with no
  mount path.

## Limits

- No run had a labelled block device mounted: the fixture is tmpfs and the workstation
  showed only `/`. Names from udev labels are covered by the unit tests of both
  adapters, which hand `volumes()` its labels through the lookup parameter the adapters
  fill from udev, and by the existing tests of the udev record decoding.
- The acceptance harness browses through the application's own adapter, which does not
  list GVfs locations. The package's copy, which the installed application uses, is
  covered by its unit and integration tests; no staged run with the installed package
  and no run with a real GVfs location was made for the names.
- The full name of a shortened drive name arrives with the drive's details, after its
  free space was read: for an offline location the dropdown and tooltip keep the short
  name (see "Full names" below).
- Windows was not run. Its presentation is unchanged by construction (its labels are
  drive letters that their roots start with), and the platform-neutral UI test above
  guards it; CI runs it on Windows.

## Full names

A drive name shortened beyond 16 columns (`Documents on fileserver` shows
`Document…eserver`) was shortened in the dropdown too, and the tooltip did not give the
full name. Now:

- The package's volume name of a GVfs location is the location's full name, from its
  folder's name in the bridge alone (`gvfs::location_name`): its label with the user,
  domain and port the name has, as the drive list adds them to locations that would
  share a name (`3249dea2`). Nothing is opened, so the location's backend is not asked.
  Other drives keep their udev label.
- When a drive's name is its volume name shortened (its start and end around `…`), the
  tooltip and the dropdown row give the volume name instead; the bar keeps the short
  name. A volume name the drive's name does not shorten keeps its own line
  (`USB1 — USB — 1.0 GiB free`).

| Command | Result |
| --- | --- |
| `cargo test --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml` | 98 unit and 27 integration tests passed; `a_gvfs_location_is_named_by_its_location_without_opening_it` asks for the volume names of locations in a bridge that does not exist and gets `Documents on fileserver` and `example.com (alice, port 2222)`; four other paths are opened as before and fail |
| `cargo test -p midnight-ui --lib --test integration` | 1733 and 1242 passed (11 ignored each), on top of `3249dea2`; `drive_names::a_shortened_name_is_spelled_out_in_the_tooltip_and_menu_once_its_volume_name_arrives` and `volume_presentation::tests::a_name_is_spelled_out_only_from_the_name_it_shortens` are new |
| `cargo clippy --features midnight-acceptance/acceptance -p midnight-ui -p midnight-acceptance --all-targets -- -D warnings`, and the package's clippy and `cargo fmt --check` | clean |

Both behaviour tests failed first: the package opened the location (`NotFound`), and
then named the SFTP location without its details (`example.com`); the UI's tooltip
began with `Document…eserver` and gave `Documents on fileserver` on a line of its own.
The unit test of the match failed with its end check removed.

