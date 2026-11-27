# Glossary

The terms that Midnight Explorer's documentation uses with a project-specific meaning.
Each term has its own anchor (the term in lowercase, hyphenated), so other pages link
to it on first use. Code identifiers can use older names; they are given in each entry.

## Panels and navigation

### Panel

One of the two file lists side by side in the main window. Each panel has its own
[tabs](#tab), history, drive bar, path bar, [panel filter](#panel-filter) and
[view](#views); commands act on the active panel, and Tab switches it. Ctrl+U swaps the
panels, and Ctrl+F11 lets the active panel fill the panels area. The documentation says
"panel" throughout; source code calls it a pane (`PaneId`, `MAX_TABS_PER_PANE`). See
[Controls](user-guide.md#controls).

### Tab

A folder tab within a [panel](#panel). Ctrl+T opens one at the current folder, Ctrl+W
closes it (the last tab stays) and Ctrl+Shift+1 to 9 switch to a numbered tab. A panel
holds at most 32 tabs, and the tabs, their order and the active one are restored after
a restart. See [Limits](limits.md#windows-and-sessions).

### Views

The three ways a [panel](#panel) shows its folder, chosen per panel: **Brief** (names in
columns, Alt+2), **Detailed** (one row per entry with its details in columns, Alt+3)
and **Thumbnails** (a grid of large icons and image or video previews, Alt+5). See
[panel views](user-guide.md#path-editor-views-and-thumbnails).

### Hot Paths

Saved locations that open with one key: folders, folders inside archives and Network
locations, at most 64. A new one takes the first free number of 1 to 9 and 0 (0 is Hot
Path 10), and paths beyond ten have no number. Pressing that digit in a focused panel
opens it, Ctrl+Alt+digit opens it in the other panel, and Ctrl+Shift+Plus adds the
current folder. The Hot Paths star on the shared toolbar lists them, and **Manage Hot
Paths…** edits them. Settings files and code call them bookmarks (`bookmarks`,
`bookmark.open.1`). See
[Views, tabs, Hot Paths and navigation](user-guide.md#views-tabs-hot-paths-and-navigation).

### Panel filter

A saved per-panel filter (Ctrl+F12) that shows only entries matching wildcard masks, a
regular expression or a metadata expression. See
[metadata expressions](developer/metadata-filters.md).

### Inline folder tree

Folders expanded in place within a [panel](#panel) with their **+** button; a
Ctrl+click on it expands the levels below as well, down to 64 levels below the panel's
own entries. See [tree behavior and limits](developer/tree-panel.md).

### Captured selection

The items and destination that a command takes at the moment it is given, such as F5's
selection and the other panel's folder. Later changes in the panels do not retarget a
running command.

### Feed to panel

A [Find](developer/find.md) button that shows the search results as a list in the
active [panel](#panel), where the panel keys (F3, F5, F6, F8 and selection) act on the
real files; Backspace returns to the folder.

### Jobs

The **Jobs** button at the right end of the bottom bar opens the list of queued,
running and finished file operations of this session, such as Copy, Move and Delete,
with each job's progress window and its pending decisions. See
[operation progress](developer/operation-progress.md).

### Custom Commands

Commands that you define in **Options › Custom Commands**: **Run directly** starts a
program with literal arguments, **Run in terminal** runs a PowerShell, POSIX shell or
Command Prompt script in a terminal, and **Separator** groups them. They appear on their
own toolbar above the command editor and in the F9 User menu, at most 128, and each can
get a shortcut as the action `user-command.<id>`. See the
[Custom Commands contract](developer/user-commands.md).

### Comparator

A rule in **Options › Comparators** that chooses how **Compare files** (Ctrl+Shift+C)
compares a pair whose names match it: **Automatic**, **Built-in text**, **Built-in
image** or an **External program**; at most 64 rules, and the first enabled match wins.
A comparison window is also called a comparator window. See
[file comparison](developer/file-comparison.md#comparator-settings).

### Salamander

Altap Salamander, a two-panel file manager for Windows whose source is published as
[Open Salamander](https://github.com/OpenSalamander/salamander) (GPL-2.0). Midnight
Explorer follows its conventions where the documents say so, for example in the parent
row `..`, Compare directories, the Make File List templates and keeping archive
associations of the panels separate from the system's. Its source files (`src/…`) are
cited as behavioral reference only; no code, assets or resource strings are copied. See
[Design and scope](user-guide.md#design-and-scope).

## Archives

### Mount

An open archive: the session in which an [archive worker](#worker) has indexed one
archive and serves its listings and member reads. Panels, tabs and viewers that show
the same archive share its mount. On Linux the word also has its usual meaning, a
mounted filesystem (`/proc/self/mountinfo`). See
[archive operation limits](archive-formats.md#current-operation-limits).

### Spool

Private files of the application in which archive members are staged when they are
decoded: anonymous files in a folder that only the current user can open. The
**spool volume** is the volume that holds them; decoding stops before its free space
falls below max(1 GiB, 2% of its capacity). See
[opening and reading archives](archive-formats.md#opening-and-reading-archives).

### Lease

1. **In the application:** a staged archive member's bytes, owned by the host and
   kept in memory or in the [spool](#spool) while something reads them; an archive
   inside an archive is read through such a lease. See
   [archives inside archives](archive-formats.md#archives-inside-archives).
2. **In development only:** a claim that one process holds on a shared machine
   resource. The build lease gives one owner a Cargo target directory; the desktop
   lease makes desktop, input and performance runs serial on a machine; the WSL lease
   does the same for Linux runs under WSL. Users never meet these. See the
   [agent workflow](developer/agent-workflow.md#desktop-and-wsl-leases).

### Archive worker protocol

The versioned messages between the host and an [archive worker](#worker). **Protocol
1** returns member bytes in its replies and keeps legacy limits, such as a 64 MiB
source. **Protocol 2** adds host-owned staging into the [spool](#spool), which archives
inside archives need. Protocol 3 adds cooperative cancellation of a member read and
protocol 4 passwords and reasons why a member cannot be read; the bundled archive
plugin uses protocol 4, and the host still serves workers of protocols 1 to 3. See the
[filesystem API](developer/filesystem-api.md).

## Plugins and processes

### Plugin

A separately built package that the application finds beside its executable, in
`viewers/`, `filesystems/` or `thumbnailers/`. Each has its own Cargo workspace under
`plugins/` and depends only on the public API and SDK. Native plugins (the text,
source and Markdown viewers and the native and Network Neighborhood filesystems) run as
trusted code inside the application; the others run as [workers](#worker). A missing
plugin removes its feature. See the [developer guide](developer/README.md).

### Viewer

A [plugin](#plugin) that F3 opens a file in: text and hex (Basic), source, Markdown,
image, PDF and HTML (Chromium); the Office preview shows its contents through the HTML
viewer, and video plays through the video [thumbnailer](#thumbnailer) package.
**Options › Plugins** chooses which viewer opens which file type. See
[Internal viewers](user-guide.md#internal-viewers) and
[viewer file types](viewer-associations.md).

### Thumbnailer

A [plugin](#plugin) in `thumbnailers/` that draws image or video previews for the
Thumbnails [view](#views): the image thumbnailer and the video thumbnailer, whose
package also holds the video playback helpers. See
[viewer file types](viewer-associations.md#thumbnailers).

### Filesystem plugin

A [plugin](#plugin) in `filesystems/` that lists and reads a kind of location: the
native filesystem of Windows or Linux (needed for any browsing), Network Neighborhood,
and the archive [worker](#worker), which opens archives as folders. See the
[filesystem API](developer/filesystem-api.md).

### Worker

1. **A restricted helper process** that parses untrusted content for the application
   and talks to it only through a versioned protocol: the archive worker, the image and
   PDF viewers, the thumbnailers and the video playback helpers. On Windows they run in
   an [AppContainer](#appcontainer). Each has bounded memory and time.
2. **A backend worker:** a thread of the application that does non-UI work, such as
   listing folders, sorting, hashing and searching, so that the window stays
   responsive.

See the [worker SDK](developer/worker-sdk.md).

### AppContainer

A Windows sandbox. The restricted [workers](#worker) run in one AppContainer without
capabilities (`MidnightExplorer.RestrictedWorker.v1`): they can open only what the
application hands them and the libraries they are granted to load. See
[the workers' grant on installed files](../SECURITY.md#the-workers-grant-on-installed-files).

### Elevated listing helper

`midnight-elevated-listing` (`.exe` on Windows), a small program installed beside the
application. When a folder cannot be opened because access was denied, **Retry with
authorization** runs it once as administrator (after the UAC prompt) or as root
(through polkit) to list that one folder. It loads no plugins and changes nothing. The
application runs it only from a folder that no other account can change. See the
[directory authorization helper](../SECURITY.md#directory-authorization-helper) and
[Troubleshooting](troubleshooting.md#cannot-open-folder-access-denied).

## Installation and releases

### Release ZIP

The portable Windows download, `midnight-explorer-<version>-x86_64-pc-windows-msvc.zip`;
the Linux download is `midnight-explorer-<version>-x86_64-unknown-linux-gnu.tar.gz`.
Extract the complete archive and run the executable; keep the `viewers`, `filesystems`
and `thumbnailers` folders beside it. An extracted copy that your account can change
updates itself. See [Download and install](../README.md#download-and-install).

### MSI installation

A copy installed with the Windows installer,
`midnight-explorer-<version>-x86_64-pc-windows-msvc.msi`: **Just for me** into
`%LOCALAPPDATA%\Programs\Midnight Explorer (MSI)` or **For all users** into
`C:\Program Files\Midnight Explorer (MSI)`. From 0.6.3 an MSI installation updates
itself through the next release's MSI. See
[Windows installer](user-guide.md#windows-installer).

### SHA256SUMS

<a id="minisig"></a>The checksum file published with each release: one
`<sha256>  <file name>` line for each archive and the MSI. `SHA256SUMS.minisig` is its
minisign signature by the project's release key, whose public half is
`packaging/update-signing.pub` and is built into the application. The updater installs
only downloads that match both. See
[Troubleshooting](troubleshooting.md#check-a-download-by-hand).

### Private installation root

A folder that no other account can change: on Windows only you, the system and
administrators; on Linux only you or root, not writable by group or others. Plugins
installed below such a root pass the strictest trust checks. `build.bat` deploys every
Windows build to a private installation in a new `%USERPROFILE%\MidnightExplorer-*`
folder, and tests that install plugin fixtures take their root from
`HOST_TEST_INSTALL_ROOT`. See [Windows x64](user-guide.md#windows-x64) and
[test execution](developer/test-execution.md#private-plugin-installation-root).

### CEF

The Chromium Embedded Framework, the browser runtime of the HTML viewer. The build
downloads a pinned CEF archive into `.tools/cef/` and checks its SHA-256; Chromium's
sandbox always stays on. See the
[HTML viewer](user-guide.md#internal-viewers).

### LTO

Link-time optimization: the compiler optimizes across crates when it links. Release
builds use fat (whole-program) LTO with one code-generation unit, which makes the final
link slow and memory-hungry. See [release settings](release-build.md#release-profile).

### ICE

Internal Consistency Evaluators: the Windows SDK's validation rules for MSI databases
(`darice.cub`). The release checks run them on the MSI and fail on findings that are
not recorded as known. See [release builds](release-build.md).
