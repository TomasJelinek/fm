# Documentation audit — 2026-11-24

## Scope

The documents of main at `ac525b6f`, after the 0.5.1 preview: every statement that
could be checked against the source (commands, shortcuts, options and defaults, limits,
file names, versions, schema numbers, paths and behaviours) in the top-level documents,
the user documents under `docs/`, the developer references under `docs/developer/`,
the package READMEs and the crate, plugin and API READMEs. Dated records under
`docs/verification/` and past release notes were read only for facts about later
behaviour and were not changed. The work was split by area between four reviewers with
separate files; each checked its documents against the source of the same commit.

Method: grep and read the code that a statement names (constants, catalog entries,
Slint components, settings codecs, CI workflow, scripts); read the commit messages of
21 to 24 November for behaviour that changed; run the repository's documentation check
(links, anchors, local paths, private strings, control bytes) over all 740 Markdown
files. No build or test ran for this audit.

## CI and release facts

Read on 24 November from GitHub (Actions and Releases of the project repository):

- Hosted CI run 45306086300 at `c8c20f49` (24 November, 01:34 UTC): 30 of 30 jobs
  passed on Windows and Ubuntu 24.04, among them `check (ubuntu-24.04)`, which runs the
  workspace tests through `scripts/ci-workspace-tests.py`, so the Linux library tests
  of `midnight-platform-linux` (the native Wayland drag and drop tests and the Run
  directly output tests) and the Linux build of `midnight-ui` passed there.
- Hosted CI run 45339190424 at `c6fc88e7` (24 November, 06:16 UTC): 30 of 30 jobs
  passed.
- The runs from `77a22d64` to `ec55023c` (23 and 24 November) failed; `c8c20f49` was the
  first to pass again. The run at `ac525b6f` was still in progress when this was read.
- Release `v0.5.1`, "Midnight Explorer 0.5.1 (preview)", marked as a pre-release, was
  published on 24 November at 05:50 UTC from `c8c20f49`, with
  `midnight-explorer-0.5.1-x86_64-pc-windows-msvc.zip`,
  `midnight-explorer-0.5.1-x86_64-unknown-linux-gnu.tar.gz`, `SHA256SUMS` and
  `SHA256SUMS.minisig`.

## Changes

Top-level and user documents:

- **FEATURE_STATUS.md:** last passing CI runs (`c8c20f49`, `c6fc88e7` instead of
  `2b901122`); the update check's 20-hour startup interval, the published signature and
  REL-04; the path editor's select-all; Shift ranges from the cursor after the anchor
  goes stale; the Delete confirmation's layout; native Wayland and Run directly Linux
  tests now pass in hosted CI (they had "not run on Linux"); `tools_checksum_unlimited`
  is written and passed on both systems on 22 November (tools record), so the Checksums
  "Open" line no longer names it; long lines rewrapped.
- **IMPLEMENTATION_BACKLOG.md:** PLAT-03 said `MIDNIGHT_WAYLAND_DND=0` turns native
  Wayland drag and drop off; since `c272312d` it is off unless `=1`
  (`drag_input_linux.rs`), and its Linux tests passed in CI. PANEL-03 named the
  acceptance harness as unable to open the Shell sheet; `panels_properties_several`
  does so now, with no recorded run. HASH-01 removed: its only gap,
  `tools_checksum_unlimited`, is written and recorded PASS on Windows and Linux
  (`20261122-100257`, `20261122-112449`). New REL-04: CI and validation never run
  `cargo test -p midnight-update --release`, so the release-only refusal of the test
  endpoint and key was checked once by hand.
- **USER_GUIDE.md:** a duplicated Move paragraph merged; native Wayland drag and drop
  only with `MIDNIGHT_WAYLAND_DND=1`; the path editor opens with the whole path
  selected; the Delete confirmation shortens a long folder as the path bar does; settings
  schemas through 40 (it said 36); the startup update check runs at most every 20 hours
  (it said once a day; `STARTUP_INTERVAL` is 20 h); every PDF page step counts
  (`ac525b6f`); Secure this folder and a folder owned by Administrators under an
  elevated token (`c8c20f49`); long lines rewrapped.
- **SECURITY.md:** settings schemas through 40 with versions 37 to 40 named (it said 36);
  Secure this folder's owner rule (`CheckTokenMembership` in `trust_repair_windows.rs`).
- **SETTINGS_FILES.md and MAKE_FILE_LIST.md:** since `f42d2e77` a confirmed replacement
  of a read-only, hidden, compressed or encrypted Windows file goes ahead and keeps those
  attributes; the settings export said such files are never replaced.
- **KEYBOARD_SHORTCUTS.md:** Reload reopens a text page where it was but an HTML page at
  its top (`worker_links.rs`, `reload`).
- **SYSTEM_REQUIREMENTS.md:** the Wayland opt-in; the update check's network access.
- **ACTIVE_WORK.md:** 0.5.1 published from `c8c20f49`; what main has since; the CI runs
  above (the page claimed a pass at `2a9e382c` that no record holds); the whole Windows
  suite at `c8c20f49` from the acceptance time record; release validation reduced to
  what is still owed.
- **README.md:** one long line rewrapped.
- **packaging/windows/README.txt, packaging/linux/README.txt:** "No automatic updater"
  replaced by a short description of Check for updates (20 hours, signed checksums,
  rollback, release page for folders only administrators or root can change); the Linux
  Wayland opt-in.

Developer documents (details in the commit messages):

- **Viewers:** WEB_VIEWER_API (narrow layout, shortcut ids, Reload position),
  VIEWER_PREFERENCES (Alt+Left/Right over an HTML page), VIEWER_PRESENTATION (the HTML
  row), VIEWER_PROTOCOL (API 0.1.0 baseline, API/SDK 0.2.0, ABI 1.0; stale sentence and
  typos removed), FILE_COMPARISON (the comparator's content check for a moved change
  time, which CONTENT_READS linked to but nothing described).
- **Engine and panels:** SELECTION (Shift ranges), PATH_PASTE (select-all),
  ENGINE_LIFETIME and TRANSFERS (an aborted copy actor ends its jobs' batches after the
  step in flight; selections of more than 64 items stream), MAKE_FILE_LIST (the dialog's
  common edit and dropdowns), CHECKSUMS (manifest replacement keeps attributes),
  TREE_PANEL (several items open one Shell Properties sheet), DIALOG_ERGONOMICS (a
  missing blank line before a heading).
- **Platform and build:** UPDATES (`midnight-explorer` in the swap, schema 40 stores the
  choices only when one is off, 0.5.1 is a pre-release, the `loopback` tests, the
  stand-in repository of the signing tests, the release-only check), README (shared
  installations load plugins; filesystem ABI 1.4), NATIVE_FILESYSTEM_API (VFS-01 label),
  TEST_EXECUTION (the idle probe's executable name), ACCEPTANCE (the harness built at
  `opt-level = 2`; `files_links_volumes` is written, not a placeholder, and now has its
  row), RELEASE_BUILD (the archive's single top folder, which the update check requires),
  archive-filesystem-development (shared installations), `api/README.md` (the tiny-viewer
  example is not built by CI, SDK-05).

## Checked without change

README feature list and versions (Rust 1.99.1, workspace 0.5.1, 26 themes); CODE_STYLE's
CI commands; every API, SDK, ABI and protocol version in FEATURE_STATUS and the API
READMEs; the shortcut catalog's viewer, link and comparison actions against
KEYBOARD_SHORTCUTS; scripts and `MIDNIGHT_*` variables named in the user documents exist; every
document under `docs/` is listed in a documentation map; every third-party package in
the 22 lock files is in the complete inventory or the lock-only list
(`docs/dependencies/`); the 0.5.1 release notes (left as published).

## Remaining

- Not read line by line for lack of time: the middle of TRANSFERS.md, FIND.md and
  TREE_PANEL.md, the rest of SCROLLING.md, most of SECURITY.md outside the sections named
  above, ARCHIVE_FORMATS.md, DRIVE_INFORMATION.md, VIEWER_ASSOCIATIONS.md and the
  provenance documents. Their identifiers and constants were sampled.
- `docs/developer/acceptance/panels.md` has no row for `panels_properties_several`; it
  needs a recorded run first.
- Code comments that name schema 39 for the update choices, which are schema 40:
  `crates/midnight-core/src/preferences.rs` and the test
  `settings_from_before_schema_39_turn_the_startup_check_on`.
- The [update check record](2026-11-23-update-check.md) names schema 39, the number of
  its own export; it stays as recorded.
