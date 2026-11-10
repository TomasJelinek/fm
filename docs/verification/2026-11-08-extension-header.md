# Extension sorting from panel headers — 2026-11-08

Both detailed panels expose **Ext** beside **Name**. Clicking it activates that
panel and invokes its existing extension sort; clicking again reverses direction.
The arrow, tooltip and accessible value reflect the same state as Ctrl+F4 and the
sort dropdown. Brief/Thumbnails continue to use the dropdown and shortcut.

The shared header maps semantic sort IDs explicitly: Name=0, Extension=3, Size=1,
Modified=2. Visual position never becomes a sort ID. Full filenames remain in the
list; Size/Modified keep their existing right-side alignment. Ext reserves at most
45% of the available filename-header space so Name remains usable at the minimum
split width, in mouse/touch modes and with font size 32.

No backend sorting, provider, path, filesystem or dependency changes. Salamander
reference: `src/mainwnd3.cpp`, `CM_LEFTEXT` / `CM_RIGHTEXT` / active extension sort,
all routed through `ChangeSortType(stExtension, TRUE)`.

The command bridge integration test clicks actual headers in both panels, checks
ascending/descending backend commands and accessibility values, uses Ctrl+F4 to
reverse again, and verifies Name/Size/Modified still dispatch their original keys.
It renders both extreme splitter positions at font 32 in mouse/touch modes and
requires Name and Ext to retain clickable widths.

Red evidence: the old UI exposes zero Extension headers instead of two. Review
also caught the uncapped first draft collapsing Name to width zero in a narrow
pane; Windows and Linux tests reproduced it before the width cap was added.

Build, test and render artifacts belong to `_build/qa/extension-header/`.

Final verification:

- Windows: `cargo test -p midnight-ui --test command_bindings --features
  slint/unstable-wgpu-30` — 2 passed, 0 failed.
- Linux/WSL: `cargo test -p midnight-ui --test command_bindings` — 2 passed,
  0 failed.
- Inspected software-renderer captures at normal density/font and the narrow
  touch panel at font 32. Name and Ext remain available, with clipped labels when
  necessary; metadata alignment is preserved.
- Scoped Rustfmt and `git diff --check` pass. Source-size check reports 1,262
  files within the 100,000-byte limit. No dependencies changed.
- Independent review caught the narrow-panel issue above; follow-up review of the
  fix and regression coverage reported no further findings.

Windows debug application build passed (`cargo build -p midnight-explorer`). The
existing 34 warnings are in unfinished tree-engine code; no release rebuild.
`_build/debug/me.exe` is 84,908,032 bytes, SHA-256
`6fe00a0e3f2d2e49e5ddd75bd55b1130da21439e92ae424978312dbf01618f34`.
Its existing plugin set is registered through the protected runtime at
`C:\Users\tjelinek\MidnightExplorer-20261108-135514-14157850`.
Native `--ui-smoke-test` startup exited 0 with empty stderr and produced
`startup.png`. User preferences were unchanged (SHA-256
`bcf7388cc3c3e2742fd86febf6f91a3edac29fb312fdfc6bc054492bcbbab053`).
