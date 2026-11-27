# Viewer file types

**Options › Plugins** has four tabs: **Viewers**, **Filesystems**, **Archives** and
**Thumbnailers**. The Viewers tab decides which [viewer](glossary.md#viewer) F3 opens
for a file; the Thumbnailers tab decides which [thumbnailer](glossary.md#thumbnailer)
draws a file's preview. The viewer and thumbnailer packages' own manifests
([manifest and installation](developer/quick-start.md#manifest-and-installation)) supply
the defaults and are never modified.

## Viewers

The **Viewer priority** list shows the installed viewers: the Text and binary viewer
(called the Basic viewer on this page), the Source, Markdown, Image, PDF and HTML viewers, and Video,
each when its package is installed. It also lists packages that could not be loaded,
with the reason, and viewers that are
no longer installed but still have saved settings ("Not installed · settings
retained"); those settings stay for a later reinstallation. A checkbox enables or
disables each viewer; a disabled or unavailable viewer opens no files.

Select a viewer and edit **Associated extensions**:

- Enter literal suffixes separated by spaces, commas or semicolons, for example
  `rs; py; custom.txt`. A leading `.` or `*.` is accepted and removed. Matching ignores
  case.
- An entry also matches a file whose whole name it is: the Source viewer's default
  `makefile` opens `Makefile`, and adding `dockerfile` to a viewer's list routes
  `Dockerfile` to it.
- A saved list replaces the viewer's defaults. It therefore does not gain extensions
  that a newer version adds to the defaults; **Reset** removes the saved list and
  restores the current defaults. For example, the Source viewer's defaults include
  `csv` and `tsv`, which it colors column by column; removing them from its list
  returns those files to the Basic viewer.
- An empty list removes all of that viewer's matches.
- Limits: at most 512 extensions for one viewer, each at most 64 bytes, without paths,
  wildcards or empty parts (`/ \ * ? : < > | " , ;` and `tar..gz` are refused); the
  typed text at most 32,768 bytes, and all saved lists together at most 65,536 bytes.
  Invalid text shows its reason under the field and keeps that viewer selected; **OK**
  refuses until it is corrected or reset.

When more than one viewer matches a file:

1. the longest matching suffix wins, so `tar.gz` beats `gz`;
2. between equally long matches, a viewer moved up the list with **Move viewer up**
   (or **Move viewer down** for the others) wins over the viewers below it.

Editing associations changes routing, not what a viewer can read. The Basic viewer
remains the fallback for files that no viewer matches, and **Alt+F3** always opens
Basic, whatever the lists say.

DOCX, XLSX, PPTX, ODT, ODS and ODP files open in the Office preview, which shows
their contents through the HTML viewer. It has no row of its own: with the HTML
viewer missing or disabled, such a file reports that the preview needs it, unless a
viewer's list names its extension.

**OK** saves these choices and **Cancel** discards them. Like enabling or disabling
a viewer, they apply after the application is restarted; the page says so. The dated
[routing design](superpowers/specs/2026-11-09-viewer-associations.md) describes the
validation and precedence rules.

## Thumbnailers

Select a thumbnailer on the Thumbnailers tab to edit its **Associated extensions** in
the same way, with these differences:

- only final extensions, such as `jpg` or `png`: at most 16 letters, digits, `-` or
  `_`, without dots, so no compound suffixes;
- at most 32 extensions for one thumbnailer, and at most 4,096 bytes of typed text;
- an empty list turns off that thumbnailer's previews, and **Reset** restores its
  installed list;
- a successful **OK** applies the change without a restart.

## Archives and rescanning

The Archives tab lists the installed archive plugins. Its **Archive associations…**
button opens the **Options › Archives** page, which chooses which file types open as
archive folders; see [archive formats](archive-formats.md#associations).

**Rescan all plugins** reads the viewer, filesystem (including archive) and
thumbnailer catalogs again. Once every catalog has answered, a line beside the button
says what it found, even when nothing changed: for example
`Rescanned: no plugins found.`, `Rescanned: found 1 viewer plugin.` or
`Rescanned: found 7 viewer, 2 filesystem, 3 archive and 2 thumbnailer plugins; 3
packages are unavailable.`, or `Rescanned; some plugin lists could not be read.` when a
catalog failed. Opening Options lists the plugins without such a line.
