# LOC03: locale-ordered navigation and compact drive buttons

Status: LOC03 focused Windows/Linux checks, UI library tests and independent review passed.
The final combined UI gate also passes: 631 tests, including the corrected
Custom Commands fixture and preview regressions, in
`_build/qa/custom-command-preview/final-ui.log`.

The later Network placement correction below supersedes the original alphabetical
Network insertion policy. The original LOC03 evidence remains historical; the two
updated boundary regressions fail against that policy in
`_build/qa/tree-budget/red.log` (positions 1 and 2 instead of 0).

## Behavior and boundaries

Drive discovery and desktop special-location discovery now prepare `LocaleService`
collation keys on their existing blocking workers. Each section sorts independently.
The comparator uses captured keys, then deterministic raw labels, paths and IDs. No
locale calls, filesystem calls or alphabetical sorting were added to UI callbacks.
The discovery watch slots, final generation checks, publication backpressure and
raw target identities remain in place. Missing locale keys use the existing raw
ordering policy; native collation failures reject that update and retain the prior
UI inventory. Key storage permits at most 4 MiB per key and 16 MiB in aggregate.

Network Neighborhood is a typed virtual choice at the boundary after all drives
and before local special folders. The desktop-context event publishes position zero
within the special-location section; Network never becomes a native navigation path
or participates in alphabetic sorting. One shared UI choice vector supplies the
dropdown and drive bar, including while initial desktop discovery is pending.
Hot Paths retain their explicit slot order and numbering. Native identity
deduplication occurs after backend sorting, so duplicate provider records cannot
shift Network into the special-folder section.
Distinct IDs or semantic kinds on the same path remain separate. Additional
OneDrive account entries retain the existing menu-only
filtering on the compact bar. Both surfaces expose the unavailable Network item
as disabled. Activation checks current availability and a revision-bearing token,
including after ordering changes, provider removal/reappearance and panel swaps.

Drive buttons now fit the measured caption origin, natural text width and existing
4/8 px density edge inset, subject to the normal density minimum target width.
The unexplained fixed 46/56 px drive minimum is removed. Icon position, caption
spacing, accessible full text and special-location/hot-path button geometry stay
consistent with their density profile.

## Reference behavior

Inspected the supplied local Salamander source:

- [drivelst.cpp:1585](D:/experiments/salamander-main/src/drivelst.cpp:1585),
  `CDrivesList::BuildData`, constructs the common inventory; Network Neighborhood
  is added at [line 2032](D:/experiments/salamander-main/src/drivelst.cpp:2032).
- [drivelst.cpp:2479](D:/experiments/salamander-main/src/drivelst.cpp:2479),
  `IncludeDriveInDriveBar`, includes the Network target; `FillDriveBar` at
  [line 2502](D:/experiments/salamander-main/src/drivelst.cpp:2502) walks the same
  inventory in order and filters the supported toolbar entries.
- Source SHA-256:
  `B93CD21C98B0B345050E740E57422DD5810CFF78E2200FCBDBBF95E4AC7E2CA8`.

The official help also describes Network in both controls and documents Hot Paths
and As Other Panel as menu entries. See [Change Drive Menu and Drive Bar](https://www.altap.cz/salamander/help/salamand/configuration_chdrv/)
and [Change Drive Menu](https://www.altap.cz/salamander/help/salamand/navigating_chdrvmenu/).
These are behavior references; alphabetic locale ordering is the user's explicit
Midnight Explorer requirement. No reference implementation or assets were copied.

## Verification evidence

All evidence and command setup are under `_build/qa/navigation-ordering/`.
Windows commands source `windows-env.ps1`: repository-local Cargo/Rustup,
`CARGO_BUILD_JOBS=2`, dev/test debug information disabled, incremental compilation
disabled, and TEMP/TMP under `_build/tmp`. Every Cargo build uses `--locked
--offline` and the immutable SDK config at
`_build/archive-streaming-api/sdk/worker-sdk-1794226812562261700/.cargo/registry.toml`.
The backend target is `_build/archive-folder-sizes/windows`; UI uses the shared
`_build` target only during explicitly coordinated ownership windows.

| Check | Result | Evidence |
| --- | --- | --- |
| Original backend ordering regressions | Five expected failures before implementation | `windows-red.log` |
| Original compact button layout | Expected failure: desktop `I:` width 46 px, measured requirement 32 px | `ui-red.log` |
| Original Network shared ordering | Expected failure: dropdown had separate `network` identity while toolbar model omitted it | `network-ui-red.log` |
| Network locale position | Expected failure: placeholder position 0 versus locale-required position 1 | `network-backend-red.log` |
| Duplicate provider identities | Expected failure: Network position 2 versus surviving-prefix position 1; final 2 helper tests and 5 integration tests pass | `dedup-red.log`, `dedup-green.log` |
| Windows navigation ordering | 5 passed | `windows-final.log` |
| Windows desktop integration | 9 passed | `windows-final.log` |
| Windows volume integration | 5 passed | `windows-final.log` |
| Windows desktop worker/generation/backpressure unit tests | 7 passed | `windows-final.log` |
| Core library strict Clippy | Passed | `windows-clippy.log` |
| Engine library and three affected integration targets Clippy | Passed with existing `dead_code`/`unused_imports` categories allowed | `windows-clippy.log` |
| Final Windows engine library/all tests Clippy after dedup correction | Passed with the same reviewed baseline allowances | `windows-final-clippy.log` |
| Native Linux final helper and affected integrations | 2 helper + 5 navigation + 9 desktop + 5 volume tests passed; qualified engine Clippy passed | `linux-final.log` |
| Real Slint Network order/availability/activation | 4 passed | `_build/qa/custom-command-preview/green-ui.log` |
| Real Slint caption geometry in both densities and larger fonts | 1 passed | `_build/qa/custom-command-preview/green-ui.log` |
| Full UI library, including locale order and Hot Path slot preservation | 603 passed | `_build/qa/custom-command-preview/green-ui.log` |
| Owned Rust formatting | Passed | `format.log` |

The engine commands are:

```powershell
. ./_build/qa/navigation-ordering/windows-env.ps1
cargo test -p midnight-engine --test navigation_ordering --test volumes --test desktop @engineArgs
cargo test -p midnight-engine --lib desktop::tests @engineArgs
cargo clippy -p midnight-core --lib @engineArgs -- -D warnings
cargo clippy -p midnight-engine --lib --test navigation_ordering --test desktop --test volumes @engineArgs -- -D warnings -A dead_code -A unused_imports
cargo test -p midnight-engine --lib navigation_ordering::tests @engineArgs
cargo clippy -p midnight-engine --lib --tests @engineArgs -- -D warnings -A dead_code -A unused_imports
```

The native Linux follow-up runs `bash _build/qa/navigation-ordering/final-linux.sh`
under Ubuntu 24.04 WSL. It sources the existing native Linux verification environment,
uses the immutable SDK config, offline locked resolution and
`_build/archive-folder-sizes/linux`, and runs the two helper tests, three affected
integration targets and qualified Clippy. The archive owner's earlier broad gate
also passed core 134 tests and engine 497 tests with one manual test ignored, plus
the affected integrations; its evidence is
`_build/archive-folder-size-sort/final-linux.log`. The helper-specific follow-up
supersedes that graph after the duplicate-provider correction.

Root ran the shared UI gate using `cargo test -p midnight-ui --lib` with
`--test network_navigation --test navigation_location_layout` alongside its CMD03
integration targets. LOC03's five integration tests and all 603 library tests
passed. The overall command exits unsuccessfully because the separate
`command_preview_stays_inside_options_and_editing_clears_stale_results` case in
`user_menu_forms_ui` still fails; no full combined-command success is claimed here.

The original five engine cases covered locale-specific `h/ch/i` ordering, the virtual
Network insertion point using those same keys, unchanged Unicode/raw paths and
aliases, section separation, calls on blocking workers, absent locale/keys,
native failures and oversized keys. Existing discovery tests cover refresh floods,
stale root replacement, blocked metadata/icons, and bounded publication channels.

Two intermediate issues were investigated separately. An old desktop assertion
assumed Home occupied the first discovery slot; it now finds the typed Home entry
and verifies the same raw path. The volume refresh test's paused-clock sleep hung
because PFS02's persistent blocking workers inhibit Tokio automatic time advance.
An earlier binary passed; diagnostic instrumentation located the stall at the
deadline sleep, consistent with pinned Tokio 1.54.1 `time/clock.rs:133`. The archive
owner changed only that test to explicit deadline advancement, retaining its
refresh bound and assertions. All five volume tests then passed. Logs preserve
the original failure, isolated timeout, passing earlier binary and diagnostic
stages. Temporary instrumentation was fully removed.

An initial new UI unit assertion compared whole generated rows, including empty
Slint images. Pinned Slint's `ImageInner::eq` returns false for two empty images.
The test now compares the identity, labels, tooltip and semantic kind that define
the navigation contract; the real component and activation tests remain intact.

Independent review found the duplicate-provider offset case and re-reviewed the
correction against its failing and passing logs. No further LOC03 finding remained.

## Review slice and limits

`loc03-slice.json` lists production files, focused tests and 16 files containing
mechanical `DesktopContext.network_position` constructor updates. Before snapshots
are in `before/`, `network-before/` and `constructors-before/`; `source-hashes.json`
records the verification snapshot. These lists are not a whole-file staging
instruction: several shared files contain other agents' work and require hunk
review. The paused-clock volume-test adjustment belongs to the archive-size slice.

No dependencies, manifests, locks or distribution notices changed for LOC03. No
git actions were performed. The focused UI tests use the real Slint component
tree and input bridge; they are not installed-application visual acceptance.
The Network label currently uses the shared English application label. Drives and
local special folders use the session's locale within their sections; Network stays
between those sections. No translation catalog was added.
