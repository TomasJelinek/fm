# Restricted workers' grant on extracted installations, 24 November 2026

Base: `1aa67529` (0.5.1 plus later work). Windows 11 Enterprise 26200, pinned Rust
1.99.1. The plugin packages are those of a release build at `ca70e1af`
(`_build\release\thumbnailers`), read but never changed.

## Report

A machine running the published 0.5.1 Windows ZIP showed no video thumbnails, and the
status bar said "Some previews are unavailable: the thumbnail helper
midnight-thumbnail-video.exe could not load its libraries (access denied):
…\thumbnailers\avcodec-63.dll".

## Root cause

- The video thumbnail helper and both playback helpers run in the workers'
  capability-free AppContainer `MidnightExplorer.RestrictedWorker.v1`
  (`S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998`).
  Windows checks that SID's access to every library the helper's loader opens.
- Only explicit entries that `scripts/build-thumbnailers.ps1`, `scripts/deploy-private.ps1`
  and `scripts/acceptance.ps1` add give it read and execute on `avcodec-63.dll`,
  `avformat-63.dll`, `avutil-61.dll`, `swscale-10.dll`, `swresample-7.dll`,
  `midnight-video-playback.exe` and `midnight-video-playback-system.exe`. `icacls` on the
  release build's `thumbnailers\avcodec-63.dll` shows that entry
  (`S-1-15-2-…-1118814998:(RX)`) and only inherited ones besides; the thumbnail helper
  executables have none.
- `scripts/package-release.py` writes the ZIP with Python's `zipfile`, which stores no
  NTFS permissions. Extracted files inherit their folder's DACL only; a folder in the
  user's profile grants AppContainers nothing (only folders such as `C:\Program Files`
  grant ALL APPLICATION PACKAGES). The loader is denied `avcodec-63.dll` and reports
  `STATUS_DLL_NOT_FOUND`, and the host names the denied library.
- The update installer also extracts release archives, so an installation that the
  scripts had granted loses the grant after an update as well.
- CI never builds the bundled video helper, and the acceptance runner added the grant
  itself after staging, so no automated run saw a package without it.

Reproduced (red) on the base with the two new tests below: a fresh extraction-like copy
of the release `thumbnailers` folder (each file written anew, so it inherits only the
private test folder's DACL) gave exactly the reported text:

```
the thumbnail helper midnight-thumbnail-video.exe could not load its libraries (access
denied): <private test folder>\Midnight Explorer\thumbnailers\avcodec-63.dll
Bundled: The video helper could not load its libraries (access denied): …\thumbnailers\avcodec-63.dll
```

The Media Foundation playback helper (`midnight-video-playback-system.exe`, no FFmpeg)
started and reported ready in the same copy without any grant: executables are opened
by the creating process, only libraries by the AppContainer.

## Fix

- `midnight-worker-platform`: `grant_worker_load(path, may_change)` checks the file with
  the existing Authz check (`worker_may_load`, as `unloadable_library` does) and changes
  nothing when the workers may load it. Otherwise, only when `may_change(path)` allows,
  it merges one explicit entry without inheritance flags that allows read and execute
  (0x1200A9) to the workers' SID into the file's DACL (`SetEntriesInAclW`, then
  `SetNamedSecurityInfoW` with the DACL's current protection), checks again and returns
  `Present`, `Added` or `Refused` (an error when the user may not change the DACL or the
  workers are still denied). `prepare_worker(executable, files, may_change)` keeps the
  outcomes for the run: a worker whose files are all loadable and each loadable file
  are only looked up later; a refused or failed file is checked again at the next launch,
  so "Secure this folder" takes effect without a restart. Linux: the workers read files
  as the user; nothing is granted.
- `midnight-plugin-host`: `prepare_restricted_worker(executable, files)` supplies the
  plugin trust rule: a file is changed only when it and every folder above it are
  private (`validate_private_path`). A shared installation is not changed.
- Callers: the thumbnail host before every helper spawn (`decode_limited`), with the
  libraries beside the helper (`worker_libraries`, the list its diagnosis already
  uses); the playback host before it pins the helper's files (`VideoProcess::launch`),
  with the declared FFmpeg libraries (`VideoInstallation::libraries`) and the helper
  executable. Together these are the seven files the scripts grant.
- `UnloadableLibrary`'s "access denied" text adds on Windows: "The file does not let the
  helpers' sandbox read it, and Midnight Explorer adds that permission only in a folder
  that only you can change: secure the installation folder or extract Midnight Explorer
  into your profile".
- `scripts/acceptance.ps1` no longer grants anything: its stages are like an extracted
  ZIP, so `viewers_video` exercises the application's grant.
- `secure_installation` and `copy_installation` already keep explicit entries that let no
  other account change a file (existing tests); the new plugin-host test shows that the
  secured folder and the profile copy get the grant at their next launch.

Other sandboxed workers: the release packages hold DLLs only for the in-process native
plugins (basic, Markdown and source viewers, native and network filesystems), which are
not sandboxed, and for the HTML viewer's CEF, whose broker is not in the workers'
AppContainer. The image and PDF viewer workers, the archive worker and the image
thumbnail helper are single executables without libraries beside them, and acceptance
stages them as plain copies; none needs the grant.

## Tests

Red on the base, green with the fix (Windows):

| Test | Base | Fix |
| --- | --- | --- |
| `midnight-thumbnail-host` `extracted_installation::an_extracted_video_helper_loads_its_libraries_in_a_private_installation` (ignored; `HOST_TEST_INSTALL_ROOT`, `MIDNIGHT_THUMBNAILERS_TEST_PACKAGE`) | failed: access denied, `avcodec-63.dll` | passed, 2×2 frame, 1.2 s |
| `midnight-media-host` `extracted_playback_helpers_start_in_a_private_installation` (ignored; same variables) | failed: Bundled access denied (Platform started) | passed, both helpers ready, 2.1 s |

New unit tests, `midnight-worker-platform` (`platform::load_grant::tests`, not ignored):

- an extracted file in a profile-like folder (user, SYSTEM, Administrators, inherited):
  denied before; `Added`; loadable after; the DACL equals the old one with exactly
  `(A;;0x1200a9;;;S-1-15-2-…-1118814998)` added before the inherited entries; the folder
  unchanged; a second call `Present` without asking and without a change;
- a file with its own protected DACL keeps its entries and protection;
- `may_change` false: `Refused`, DACL unchanged, still denied;
- a file whose permissions the user may not change: `PermissionDenied`, unchanged;
- `prepare_worker`: refusals are checked again, a prepared worker neither lists nor
  checks its files again, another worker checks only files not yet found loadable.

`midnight-plugin-host`
`installation_sharing::tests::windows::the_workers_grant_is_added_only_where_the_installation_is_private`:
a shared (drive-root-like) installation is not changed; its profile copy gets exactly
one entry at its first launch while the source stays unchanged; after
`secure_installation` the next launch adds it, the folder stays private, and securing
again keeps it.

The existing real-worker test `a_worker_whose_library_is_missing_exits_without_a_system_dialog`
now grants through `grant_worker_load` (its own `SetEntriesInAclW` helper is removed):
the denied library is named, and after the grant the loader opens it.

Runs (Windows, `HOST_TEST_INSTALL_ROOT` a new owner/SYSTEM/Administrators folder below
the profile, removed afterwards):

| Binary | Result |
| --- | --- |
| `midnight-plugin-host` lib | 134 passed, 7 ignored |
| `midnight-thumbnail-host` lib | 51 passed, 5 ignored |
| `midnight-thumbnail-host` integration | 24 passed, 11 ignored |
| `midnight-media-host` lib | 44 passed |
| `midnight-worker-platform` lib | 37 passed, 1 ignored |
| `midnight-worker-platform` integration binaries | all passed |
| `scripts.tests.test_acceptance_windows`, `test_restricted_worker_identity` | 16 passed |

Acceptance (Windows, desktop lease, the release build's plugin packages copied into a
private run folder; the runner now grants nothing):
`scripts/acceptance.ps1 --only viewers_video --plugins <copy> --test-binary <suite>`:
`viewers_video` PASS in 40.2 s (thumbnails of the plain and H.264 MP4s and their
NTFS-compressed copies, F3, seeks and Play, through the bundled and Media Foundation
helpers), so the application's own grant made the staged FFmpeg libraries loadable.

Checks: `cargo clippy --workspace --all-targets --locked --offline --features
midnight-acceptance/acceptance -- -D warnings` clean; `cargo clippy --target
x86_64-unknown-linux-gnu -p midnight-worker-platform -p midnight-plugin-host -p
midnight-thumbnail-host -p midnight-media-host --all-targets --locked --offline -- -D
warnings` clean; `cargo fmt --all -- --check` clean; `scripts/check-source-size.ps1`:
3481 files, all at most 100,000 bytes.

## Not covered

- CI builds no bundled video helper, so the two extracted-installation tests run only
  where a `thumbnailers` package exists; `docs/RELEASE_BUILD.md` adds them to the checks
  before publishing, against the unpacked ZIP.
- An administrator's installation that grants AppContainers nothing (not `Program
  Files`) still fails, now with the guidance; an administrator adds the entries with
  `icacls` (SECURITY.md).
