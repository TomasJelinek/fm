# Viewer file types

Open **Options > Plugins** to select a viewer and edit which extensions it handles.
Enter literal suffixes separated by spaces, commas or semicolons, for example
`rs; py; custom.txt`. Leading `.` or `*.` is accepted and normalized. Matching
ignores case. An entry also matches a file whose whole name it is: the Source
viewer's default `makefile` entry opens `Makefile`, and adding `dockerfile` to a
viewer's list routes `Dockerfile` to it. **Reset** removes that viewer's override.

The Source viewer's defaults include `csv` and `tsv`, which colour each column;
removing them from its list returns those files to the Basic viewer. A list saved for
it before they were added does not gain them; **Reset** restores the defaults.

The list includes text, source, Markdown, image, PDF and HTML viewers when installed.
Move a viewer higher in the priority list to prefer it when more than one viewer
matches the same suffix. A longer compound suffix takes precedence over a shorter
suffix. Editing associations changes routing, not what a decoder can actually read.

An empty extension list removes that viewer's explicit matches. The installed
Basic viewer remains the fallback for unmatched files; **Alt+F3** always uses Basic,
independent of extension overrides. Disabled or unavailable plugins cannot handle
files. Preferences for missing plugin IDs remain stored for later reinstallation.

**OK** saves these choices; **Cancel** discards them. As with plugin enablement,
restart the application to apply them. Plugin manifests are never modified.

The [routing design](superpowers/specs/2026-11-09-viewer-associations.md) describes
validation, precedence and the separation between preferences and plugin capabilities.

The Plugins page also lists installed filesystem and thumbnailer packages in
separate tabs. Rescan all plugins refreshes all three catalogs from one action.
