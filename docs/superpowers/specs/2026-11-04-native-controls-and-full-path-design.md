# Native control icons, complete navigation choices and command path

This increment implements the user's requests for drive tooltips, host icons
where suitable, identical drive/special-location availability in the quick strip
and dropdown, and the complete active command directory without elision.
The verified starting point is commit `d4ef58a` (Midnight Explorer 0.1).

## Navigation choices

Both navigation surfaces use one identity-bearing choice snapshot. Include
every available drive and every quick special location: Home, Desktop,
Documents and Downloads where supported. Preserve distinct semantic aliases
even when they point at the same native directory. Labels are presentation;
selection resolves an opaque identity/revision to the original raw path.

Changing choice identities, ordering or targets invalidates an old popup or
pressed delegate. Metadata and icons may update without retargeting an action.
A volume choice retains that pane's remembered directory on the volume;
special choices navigate to their discovered native target. Backend-provided
paths remain `PathBuf`/`OsString`; no display-string round trip is introduced.

Quick buttons and corresponding menu rows share complete tooltips. Drives show
their root, native volume label and free capacity when available. Special
locations show their name and full path. Loading or unavailable capacity must
not be presented as zero. Expanded drive labels retain label and free space.

## Host icons

Extend the existing platform-neutral icon key with semantic action and location
roles. Reuse the bounded asynchronous icon worker and cache; no discovery,
Shell query, file read or raster decoding runs in `midnight-ui`. Prefetch the finite
action palette and discovered location kinds. Keep per-drive native identities.

The named action palette covers navigation, bookmarks, tabs, terminal, options,
Run, commander F1–F10, dialog actions and sorting direction. Named Slint fields
avoid coupling UI meaning to an array index. Standard widget chevrons retain
their normal widget rendering.

Windows uses appropriate Shell stock icons and trusted known-folder IDs.
Known-folder PIDLs use a narrow RAII owner and the documented allocator pair;
no arbitrary DLL resource indices or untrusted file contents are extracted.
The [SHGetKnownFolderIDList contract](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderidlist)
defines the returned PIDL lifetime. Linux uses a fixed semantic host-theme name
allowlist through the existing bounded PNG loader. Platform implementations
and tests remain in their respective files/crates.

Native action misses are negative-cached and leave the UI's semantic vector
fallback in place. A generic file bitmap is not an action fallback. Missing
special-location art may use the native folder fallback; if that is unavailable,
retain the distinct application vector for its location role. Preserve native RGB
and alpha; tint only the application's monochrome vectors. Unsupported native
roles do not change an action's availability or enable future write operations.
Bottom function buttons vertically center their icon and label together while
preserving the current heights, icon sizes and native image aspect ratios in
both density profiles and at large fonts.
The Windows adapter also rejects the observed uniformly opaque black placeholder
for semantic action/location icons. This narrow rule does not reject native file
or drive icons, black glyphs with transparent margins, or colored native art.

Native context discovery is coalesced on the backend. A changed volume inventory
requests fresh typed navigation metadata, including Windows drive letters, even
if a previous locations call is still in flight. UI code does not classify OS
path prefixes or perform native discovery. At most one context lookup runs;
stale generations cannot publish over a newer requested context.

## Complete command directory

Remove the fixed 180/200 px, elided directory prompt. Keep its complete text
beside the command editor in the same single horizontal row.
The row keeps its shell picker, Options and Run controls and usable editor
width. Its directory always follows the active pane's committed/displayed path.

Ordinary paths use their natural text width. For extreme lengths, constrain only
the viewport width and allow horizontal scrolling through the entire text; retain
every character and never impose a data/path length cap. Keep the editor and
panels usable at the minimum window size and largest supported fonts. Tooltips
and accessibility expose the complete text as well.

Keep the ordinary 480 px minimum-height floor. At large fonts, derive a larger
effective minimum from fixed control bands, one readable directory line and
one complete file row. Reserve possible pane-error and important-feedback bands
unconditionally so an error does not resize the window. This layout constraint
does not depend on path length or the scrollable text's preferred size.

File/folder rows deliberately have no hover tooltips. Control, drive and header
tooltips remain. Running sizes use amber/bold text; finished partial results
retain `≈` with ordinary styling, distinct from an active scan. F1 documents the
meaning of partial totals when access is denied or entries are skipped.

Inserted paths/names remain bare when they are simple literal shell arguments.
Other tokens use escaped double quotes, without enclosing apostrophes. Preserve
Unicode spelling, including PowerShell numeric-looking names and smart quotes;
reject ambiguous CMD literals rather than changing their target identity.

Behavior reference checked in Open Salamander: `src/editwnd.cpp:1734`
(`SetDirectory`), `:1752` (`GetNeededHeight`), `:1757` (`ResizeChilds`) and `:337`
(`InsertText`). Its prompt and editor share one text row, with separate prompt
and command content. Midnight Explorer keeps that compact arrangement while preserving full
Unicode/native identities and horizontally accessible long paths. The C++/ANSI
implementation is a behavior reference only; no source is copied.

## Verification

Test quick-strip/dropdown parity, same-path aliases, raw Unicode/native targets,
remembered volume paths and stale choice revisions. Exercise actual hover,
pointer and keyboard input. Verify RGB host icons remain colored, unsupported
actions use semantic vectors, and native lookup failures are bounded/cached.
Test full paths with long unbroken components, combining marks and emoji,
active-pane changes, narrow windows at the effective supported minimum, both
densities and large fonts. Assert a readable directory line and complete file row,
including error/important-feedback states. Keep size
updates at 250 ms and preserve row order during progress/completion.

Run the affected tests, full Windows/Linux workspace tests, formatting, strict
all-targets Clippy, source-size guards and application builds. Inspect and launch
the native Windows UI before committing this increment. Generated output stays
in ignored `target/` and `.tools/` directories.
