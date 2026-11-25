# Feature status

This page says what Midnight Explorer 0.5.3 implements in the current source, per
feature area and platform, which document describes each area, and where it is
verified. Planned work and known limits are rows of the
[implementation backlog](IMPLEMENTATION_BACKLOG.md), named here by their IDs. The
[current checkpoint](ACTIVE_WORK.md) names the commits that the latest recorded runs
used.

How to read it:

- **Platforms.** Windows x64 and Linux x86-64 are implemented. The recorded runs used
  Windows 11 and Ubuntu 24.04 under WSL 2; other systems and desktops are listed in
  [system requirements](SYSTEM_REQUIREMENTS.md) and PLAT-02. macOS is not implemented
  (PLAT-01): the application does not build there, and only the independent native
  filesystem plugin has macOS source.
- **Verified** names the acceptance scenarios that exercise an area through the real
  application (see the [acceptance suite](developer/ACCEPTANCE.md)). Their per-check
  statuses, with run ids, platform and scale, are kept in the acceptance records linked
  in each section. Dated records under `docs/verification/` are named where they hold
  the evidence. A test, scenario or CI job that exists is not evidence that it passed;
  only a recorded run is.
- Release packages include this file. Its links point into the source repository.

## Platforms, builds and packages

- **Windows and Linux:** `build.bat` and `build.sh` build the application together with
  every plugin package; release packages contain every viewer, filesystem and
  thumbnailer package.
- **Linux specifics:** native Wayland drag and drop with other applications is new, not
  yet qualified on a compositor and off unless `MIDNIGHT_WAYLAND_DND=1` (PLAT-03). On
  Ubuntu 23.10 and later the HTML viewer needs an AppArmor profile, which it offers to
  install; Chromium never runs without its sandbox.
- **Open:** macOS (PLAT-01); native and manual qualification on other distributions,
  desktops, physical GPUs and live removable or network mounts (PLAT-02); CI jobs for
  the HTML viewer and the bundled-FFmpeg video helper (CI-01); native tests in the
  release profile (REL-02); signed Windows release binaries (REL-03).
- **Documents:** [user guide](USER_GUIDE.md#build-and-run) (Build and run),
  [system requirements](SYSTEM_REQUIREMENTS.md), [release build](RELEASE_BUILD.md),
  [test execution](developer/TEST_EXECUTION.md).
- **Verified:** the last hosted CI runs recorded as passing every job on Windows and
  Linux are at `c7ef1e5c` (the `v0.5.3` tag) and `a5e5b1a4`
  ([release record](verification/2026-11-25-release-0.5.3.md#ci));
  CI times are in the [CI time record](verification/2026-11-21-ci-time.md).
  Acceptance runs on Windows are listed in the
  [Windows acceptance record](developer/acceptance/windows.md); the area records below
  list the runs on both systems.

## Updates

- **Windows and Linux:** the toolbar's update button and About's Check for updates open
  the update dialog: check GitHub's releases (previews included by default), download in
  the background on Download and install, verify the minisign signature of
  `SHA256SUMS` with the built-in release key, the archive's SHA-256 and the staged
  `BUILD-INFO.json`, and install on Restart now, restoring the previous version when the
  new one does not confirm its start. Options › General › Updates: Check for updates at
  startup (on; at most once every 20 hours) and Include preview releases (on); settings
  schema 40. Releases from 0.5.1 on are signed with the owner's release key, whose
  public half is built in; the published 0.5.1 carries `SHA256SUMS.minisig`.
- **Open:** copies that only administrators can change are not updated in place; the
  release-build check of the test endpoint is not run by CI (REL-04).
- **Documents:** [updates](developer/UPDATES.md), [user guide](USER_GUIDE.md#updates),
  [security](../SECURITY.md#update-check), [release signing](RELEASE_BUILD.md#signing-the-release).
- **Verified:** [update check record](verification/2026-11-23-update-check.md).

## Panels and navigation

- **Windows and Linux:** two panels with tabs; history (Alt+Left, Alt+Right and the
  toolbar menus); drives and mounted volumes; a path bar with clickable segments and a
  path editor (Ctrl+G, or a click after the path), which opens with the whole path
  selected, keeps a pasted path exactly as copied and opens it with Enter or its Go
  button, where a file path opens its folder with the cursor on the file; parent
  navigation that keeps the cursor on the folder left; Brief, Detailed and Thumbnails views (Alt+2, Alt+3, Alt+5); locale-aware sorting; quick
  search by typing; hidden files (Ctrl+H); Hot Paths, numbered Hot Path keys and saved
  tabs; folder sizes; native change watches with a 15-second check; free space that
  follows jobs, focus and (on internal disks) a 5-second read; startup folders
  from the command line; size-warning and error notices with details.
- **Open:** the startup tab restore replaces a panel that already shows the saved
  folder (TABS-01); actions with nothing selected and the cursor on no entry do nothing
  silently (NOTICE-03); watch gaps after failed archive or network navigation
  (WATCH-01); Ctrl+letter shortcuts on layouts without Latin letters (INPUT-01).
- **Documents:** [keyboard shortcuts](KEYBOARD_SHORTCUTS.md),
  [user guide](USER_GUIDE.md#controls) (Controls),
  [parent navigation](developer/PARENT_NAVIGATION.md),
  [path paste](developer/PATH_PASTE.md), [pixel scrolling](developer/SCROLLING.md),
  [panel diagnostics](developer/PANEL_DIAGNOSTICS.md),
  [settings files](SETTINGS_FILES.md).
- **Verified:** `startup`, `startup_paths`, `navigation`, `files_navigation`,
  `panels_paths`, `panels_root_keys`, `panels_tabs`, `files_tabs`, `panels_view_sort`,
  `files_sorting`, `panels_scrolling`, `panels_hot_digits`, `files_hot_paths`,
  `panels_notices`, `panels_busy_ops`, `files_busy_folder`, `folder_sizes`,
  `tools_folder_sizes`; statuses in the [panels record](developer/acceptance/panels.md).

## Selection, search and filters

- **Windows and Linux:** Insert, Space, Shift and Ctrl selection (a Shift range starts at
  the cursor once the cursor has moved away from the last clicked row); rectangle selection
  in every view, in archives and in trees; selection by mask (Ctrl+=, Ctrl+− and the
  keypad + − *); the panel filter (Ctrl+F12 and the toolbar funnel) with masks, regular
  expressions and metadata expressions, recent filters and an active-filter indicator;
  totals of the selected sizes in the status bar.
- **Documents:** [panel selection](developer/SELECTION.md),
  [metadata filters](developer/METADATA_FILTERS.md),
  [keyboard shortcuts](KEYBOARD_SHORTCUTS.md).
- **Verified:** `files_selection`, `panels_selection_more`, `panels_search_filter`,
  `files_filter_search`; statuses in the [panels record](developer/acceptance/panels.md).

## Inline folder trees

- **Windows and Linux:** folders expand inline in the panel listing; Ctrl+click expands
  a whole branch, at most 64 levels deep, and says where and why it stopped. Prefix
  search, filters and sorting apply to the loaded rows. F3, F4, F5, F6, F8, F2,
  Attributes, the clipboard and Custom Commands act on native tree rows, and native
  watches keep expanded branches.
- **Open:** branches do not expand archive files, archive panels have no watches,
  archive rows cannot go to the clipboard or to Custom Commands, and one tree holds at
  most 10,000 rows (TREE-01); a busy capture pool is reported as unavailable instead of
  retried (TREE-02); each changed branch costs one round trip (TREE-PERF-01).
- **Documents:** [tree panel contract](developer/TREE_PANEL.md),
  [inline trees](developer/INLINE_TREES.md).
- **Verified:** `files_tree_ops`, `tree_expand_all`, `panels_tree_keys`; statuses in the
  [panels record](developer/acceptance/panels.md) (Inline folder branches).

## Copy and Move

- **Windows and Linux:** F5 and F6, Ctrl+C, Ctrl+X and Ctrl+V, and drag and drop share
  one verified background queue. Each copied file is read back and compared by SHA-256
  by default; a sampled check can be chosen instead. Name conflicts offer Skip, a new
  name, folder Merge and Replace, which keeps the displaced original recoverable. One
  Links choice, defaulted from the destination, copies the files that links point to or
  keeps links as links; on Windows a junction is recreated without privilege, while a
  symbolic link needs Developer Mode or administrator rights. A Move on one volume
  renames without reading contents; across volumes it goes wherever Copy goes and each
  file is copied and verified as Copy does before its source is removed. Copy and Move
  take up to 1,000,000 selected items: a large selection starts copying its first item at once, totals the rest while it copies and opens each
  item only when it reaches it. I/O errors pause for Retry, Skip, Skip all or Cancel.
  "Preserve times and attributes" (on unless Options turns it off) gives copied files and
  folders their sources' times (Windows: created, modified, accessed; Linux: modified,
  accessed) and attributes (Windows: Read-only, Hidden, System, Archive, Not content
  indexed; Linux: permission bits without set-ID bits), folders after their contents
  and Read-only last; archive members keep their recorded modified time; any number of
  items; what a destination cannot store is a warning of the job.
- **Linux specifics:** Copy and cross-volume Move write only to filesystems with
  `O_TMPFILE` (ext4, XFS, Btrfs, tmpfs); FAT, exFAT, NTFS, NFS, SMB, sshfs and GVfs
  destinations are refused.
- **Open:** ownership, ACLs, extended attributes, streams, Linux creation times, a kept
  link's own times and hard links are not preserved; Paste and drag and drop copy
  content only (COPY-03); a Move from another drive
  keeps a Windows source with named streams (other Windows sources, from a USB drive or a
  share included, are removed after their verified copy), and on Linux removes only sources
  the user owns on a local filesystem; providers have no Move (MOVE-01); an archive
  copy-out of a large selection admits its members one by one: about 20 s before the
  first of 100,001 archive members is written (COPY-06); a job runs one native step at a
  time (IO-01); one namespace turn serves all jobs (IO-02); the shared disk-I/O gate can
  stall behind one hung call (IO-03); Linux syncs the folder after each copied file
  (PERF-05).
- **Documents:** [transfers](developer/TRANSFERS.md),
  [transfer options](developer/TRANSFER_OPTIONS.md),
  [disk operations](DISK_OPERATIONS.md),
  [security requirements](../SECURITY.md#verified-clipboard-copying-and-the-copy-contract).
- **Verified:** `copy`, `move`, `copy_move_delete`, `files_copy_kinds`,
  `files_move_kinds`, `files_copy_options`, `files_conflicts`, `files_links`,
  `files_links_volumes`, `files_transfer_errors`, `files_archive_transfers`,
  `files_clipboard_drag`, `journeys_links_readonly`, `journeys_filtered_conflicts`,
  `journeys_concurrent_jobs`; statuses in the
  [transfers record](developer/acceptance/transfers.md) and the
  [journeys record](developer/acceptance/journeys.md).

## Delete, Trash, create, rename and properties

- **Windows and Linux:** Delete and F8 move the selection to the system Trash (the
  Recycle Bin on Windows); with Shift they delete permanently. The confirmation asks its
  question first and lists the details below it, with a long folder path shortened as
  in the path bar. Each entry that cannot
  be removed asks Retry, Skip, Skip all errors or Cancel. F7 creates folders, including
  missing parent folders; Shift+F4 creates an empty file and asks before it replaces
  one. F2 renames, with the shared overwrite flow. Attributes, Times and Case edit the
  selected entries, also recursively, after a review step. Alt+Enter opens Properties
  (the Shell dialog on Windows, one for all selected items; a summary on Linux).
- **Open:** on Windows, Trash or Rename of a folder that this application itself holds
  open is refused (TRASH-01); a Linux Properties rename that leaves its staging folder
  counts the item as failed (META-04); Linux Rename and permanent Delete on live vfat,
  exFAT, ntfs3 and CIFS mounts are not verified (PLAT-02).
- **Documents:** [user guide](USER_GUIDE.md#controls) (Controls),
  [operation progress](developer/OPERATION_PROGRESS.md),
  [security requirements](../SECURITY.md#directory-comparison-filters-and-metadata-editing).
- **Verified:** `delete`, `trash`, `files_delete`, `files_trash`, `mkdir`, `newfile`,
  `files_create`, `rename`, `files_rename`, `create_rename`, `attributes`, `case`,
  `files_attributes`, `panels_times`, `properties`; statuses in the
  [transfers record](developer/acceptance/transfers.md) and the
  [panels record](developer/acceptance/panels.md).

## Operation progress and Jobs

- **Windows and Linux:** each Copy, Move and Delete job has its own modeless progress
  window with totals after discovery, a speed chart, a time estimate, Pause and Resume,
  and a cancel question that pauses the job while it is open. Jobs lists the session's
  jobs until they are cleared, opens their windows and answers their waiting questions.
- **Open:** measured-only refresh costs of large job and row lists (UI-PERF-01).
- **Documents:** [operation progress](developer/OPERATION_PROGRESS.md).
- **Verified:** `jobs`, `files_transfer_jobs`, `tools_jobs_copy`, `tools_notices_jobs`,
  `journeys_concurrent_jobs`, `ui_polish_jobs`; statuses in the
  [transfers record](developer/acceptance/transfers.md), the
  [tools record](developer/acceptance/tools.md) and the
  [UI polish record](developer/acceptance/ui-polish.md).

## Drag and drop and the clipboard

- **Windows and Linux:** files dragged between the panels move on a proven same volume
  and are copied otherwise (Ctrl forces Copy, Shift forces Move); drops from other
  applications are copied. A drag leaving the window goes to the other application at
  once; released within 0.4 s of the press it is cancelled there too. Windows uses OLE
  with a Shell data object and the Shell drag image, and takes drops from .NET programs;
  Linux uses X11 or XWayland XDND and the X11 file clipboard, and under native Wayland,
  with `MIDNIGHT_WAYLAND_DND=1`, the window's own data device. Archive and Network rows
  say why they do not drag.
- **Open:** native Wayland exchange is implemented but not run on a compositor; its unit
  tests and the Linux build pass in hosted CI on Ubuntu 24.04 (PLAT-03); drops from other applications
  land in a panel's current folder, never in a folder row.
- **Documents:** [keyboard shortcuts](KEYBOARD_SHORTCUTS.md),
  [security requirements](../SECURITY.md#file-drag-and-drop).
- **Verified:** `files_clipboard_drag`, `journeys_pointer_only`; statuses in the
  [transfers record](developer/acceptance/transfers.md) (Clipboard and drag and drop);
  drags to and from other Windows applications by real mouse in the
  [drag and drop record](verification/file-drag-drop.md#2026-11-23-dragging-to-and-from-other-applications).

## Find

- **Windows and Linux:** Ctrl+F or Alt+F7 open a modeless Find window, up to four at
  once. It finds names by masks, regular expressions or metadata expressions, text
  (literal, or a regular expression within each line), files modified within a period
  and identical files, in several search folders and inside archives through the
  installed archive plugins. A search keeps at most 100,000 results. Results sort by
  column and offer Go to file, View, Edit, Open and Feed to panel.
- **Open:** Find cannot start in a panel that shows an archive, and an archive inside
  an archive is matched by name but not searched (FIND-01).
- **Documents:** [Find contract](developer/FIND.md).
- **Verified:** `find`, `find_by_mask`, `find_query_syntax`, `find_roots_windows`,
  `find_result_actions`, `find_feed_panel`, `find_budgets`, `find_layout`,
  `find_archives`, `find_viewers`, `tools_find_criteria`, `tools_find_folders_help`,
  `tools_find_results`, `journeys_find_to_checksums`; statuses in the
  [Find record](developer/acceptance/find.md).

## Viewers

- **Windows and Linux:** F3 opens the installed viewer for the file's type: images
  (PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, SVG, AVIF), PDF with page layouts, Markdown,
  source code with syntax highlighting, HTML in embedded Chromium as a small browser
  (Back, Forward, Reload, the page's title and cursor; offline with its JavaScript
  running and contained without network, local stylesheets, fonts and images also of
  saved pages and of generated documentation's parent folders; Online on request), the
  Office preview of DOCX, XLSX, PPTX, ODT, ODS and ODP
  contents (shown through the HTML viewer), video playback, and the basic text and hex
  viewer. Alt+F3 forces text. Viewers search (Ctrl+F), wrap, select and copy text (Ctrl+A
  selects the whole file in text, source and Markdown views, and Ctrl+C copies up to
  64 MiB of its text, read in the background with progress; the hex view selects the
  rows shown), zoom images and PDFs, and step through the panel's files with Space and
  Backspace. Viewer choices persist. Options › Plugins assigns file types to viewers.
- **Links:** Markdown and HTML documents follow a link when it is clicked: a fragment
  scrolls to its heading, a file that a viewer shows opens in the same window, a folder
  or any other file is shown in the active panel, and web and mail links open with the
  system's default handler; other schemes are refused. Alt+Shift+Left and
  Alt+Shift+Right, or the mouse's Back and Forward buttons, go back and forth through
  the documents opened from links; in HTML also the Back and Forward buttons and
  Alt+Left/Right while there is a page in that direction.
- **Video decoding:** a bundled FFmpeg profile decodes VP8, VP9, Theora, FFV1 and raw
  video; other codecs use Media Foundation on Windows and the distribution's FFmpeg or
  GStreamer on Linux.
- **Open:** documented bounds such as loaded-page copy, the first frame of animated
  images and Markdown checkpoint limits (VIEW-01); in a document opened from an
  archive, links to files above its folder, to folders and to members that no viewer
  shows only say so on the status line (VIEW-07); a stored PDF zoom is not restored
  (each PDF opens in Fit), and no check restarts the application to compare viewer
  choices (VIEW-03); on Windows the HTML provider wakes about 64 times per second while
  a document is open (VIEW-04); some readers still treat a change of the change time
  alone as a change (VIEW-05); image decode notes and failure kinds need image viewer
  API 0.4.0 (VIEW-06); each HTML document starts its own browser, about 0.9 s
  (VIEW-08); HTML wheel glides can end early on a loaded machine (VIEW-09); the HTML
  JavaScript choice lasts for one viewer window, with no Options setting yet, and an
  offline page's scripts cannot use local storage, cookies, workers or `fetch` of its
  own files (the sandbox that keeps them from reading files); text
  scrolling pacing is not measured on Linux or with a physical wheel, and rows longer
  than one layout show in pieces (VIEW-PERF-01, SCROLL-02).
- **Documents:** [viewer presentation](developer/VIEWER_PRESENTATION.md),
  [viewer preferences](developer/VIEWER_PREFERENCES.md),
  [viewer scrolling](developer/VIEWER_SCROLLING.md),
  [viewer associations](VIEWER_ASSOCIATIONS.md),
  [HTML viewer](developer/WEB_VIEWER_API.md),
  [image and PDF viewers](developer/IMAGE_VIEWER_API.md),
  [content reads](developer/CONTENT_READS.md),
  [video worker guide](../plugins/midnight-thumbnail-video/README.md).
- **Verified:** `viewers_files`, `viewers_controls`, `viewers_selection`,
  `viewers_scroll`, `viewers_presentation`, `viewers_office`, `viewers_video`,
  `viewers_combinations`, `find_viewers`, `ui_polish_viewers`; statuses in the
  [viewers record](developer/acceptance/viewers.md). Dated measurements:
  [PDF scrolling](verification/2026-11-21-pdf-scrolling.md),
  [prepared text window](verification/2026-11-21-prepared-text-window.md),
  [long rows](verification/2026-11-22-long-rows-row-groups.md),
  [source highlighting](verification/2026-11-22-source-highlighting.md),
  [Linux video decoders](verification/2026-11-21-linux-system-video-decoders.md).

## File and folder comparison

- **Windows and Linux:** Ctrl+Shift+C compares two files: text side by side, images,
  and a streamed hex comparison of any other pair. Options › Comparators chooses the
  comparator for each file type. Compare directories selects the differences as
  Open Salamander does (size, time with tolerance and daylight saving, attributes,
  contents, subfolders, ignore masks), with progress and Cancel in its dialog, the
  result below the active panel and per-row match badges on the compared rows and,
  with subfolders, in the panels' inline subfolders (0.5.1); the result line's ×,
  Escape outside filename search, Ctrl+Shift+F11 and Clear comparison marks remove them.
- **Open:** the time to the first frame is not measured on Linux (COMPARE-01); image
  and hex comparisons report a change of the change time alone as a change
  (COMPARE-02).
- **Documents:** [file comparison](developer/FILE_COMPARISON.md),
  [compare directories](developer/COMPARE_DIRECTORIES.md).
- **Verified:** `viewers_compare`, `tools_compare_dirs`, `options_comparators`; statuses
  in the [viewers record](developer/acceptance/viewers.md) and the
  [Options record](developer/acceptance/options-tools.md).

## Thumbnails

- **Windows and Linux:** the Thumbnails view shows images, video frames, the first page
  of PDFs and archive members, decoded in restricted workers and kept in a private
  cache. Options › Plugins assigns file types to thumbnailers.
- **Open:** archive-member thumbnails have no acceptance check (MEDIA-01); video
  decoding is not qualified on Linux systems other than Ubuntu under WSL (MEDIA-02);
  reuse of thumbnails after scrolling back and after a restart (MEDIA-03).
- **Documents:** [worker SDK](developer/WORKER_SDK.md),
  [thumbnail API](../thumbnail-api/README.md),
  [video worker guide](../plugins/midnight-thumbnail-video/README.md).
- **Verified:** `viewers_thumbnails`, `viewers_video`; statuses in the
  [viewers record](developer/acceptance/viewers.md) (Thumbnails, Video).

## Archives

- **Windows and Linux:** installed archive workers browse ZIP, 7z, GZIP, XZ, Bzip2,
  Zstandard and TAR, also compressed. F3 views members, F5 copies them out through the
  verified queue (with each member's path inside the archive, file and byte totals, a
  determinate bar and an estimate), archives inside archives open up to four levels deep and
  are kept with their whole path by history, Hot Paths and saved tabs, and
  password-protected ZIP and 7z archives ask for their password. Archives are
  read-only, and RAR is not supported.
- **Open:** ARCH-01 to ARCH-09 (content limits, nested archives, writes, Copy permits
  of failed sessions, provider content leases, solid 7z decoding, stopped workers
  during a Copy, validation of the held archive file). Two archives can be open at
  once; opening a third offers to close one of them.
- **Documents:** [archive formats](ARCHIVE_FORMATS.md),
  [archive API](developer/FILESYSTEM_API.md),
  [security requirements](../SECURITY.md#archive-passwords).
- **Verified:** `viewers_archives`, `viewers_passwords`, `files_archive_transfers`,
  `journeys_archive_roundtrip`, `find_archives`, `options_archives`; statuses in the
  [viewers record](developer/acceptance/viewers.md) (Archives, Archive passwords).
  Dated measurement: [archive copy-out](verification/2026-11-22-archive-copy-out.md)
  (Windows only: about 165–200 small members per second in a release build, and Cancel
  of a copy of 100,001 members ends the job in about half a second).

## Network Neighborhood

- **Windows and Linux:** a discovery plugin (the Windows networking functions; GIO on
  Linux) lists computers and shares, opens a share when asked, and keeps saved tabs, Hot
  Paths and the ancestor parts of the path bar.
- **Linux:** explicit activation of an SMB device or share can request credentials
  in the app and retry the original target. Passwords are not saved.
- **Open:** no run against a real SMB server or SMB and FUSE shares, no in-app
  sign-in on Windows, and no macOS adapter (NET-01). Other remote filesystems
  remain separate (REMOTE-01).
- **Documents:** [Network Neighborhood contract](developer/NETWORK_FILESYSTEM.md),
  [security requirements](../SECURITY.md#network-neighborhood-discovery).
- **Verified:** plugin and host tests; dated records:
  [Network Neighborhood](verification/2026-11-09-network-neighborhood.md),
  [Linux discovery](verification/2026-11-13-linux-network-discovery.md),
  [Linux sign-in](verification/2026-11-24-network-sign-in.md),
  [network Hot Paths](verification/2026-11-11-network-hot-paths.md). No acceptance
  scenario covers it.

## Checksums

- **Windows and Linux:** Ctrl+F8 opens a modeless Checksums window: SHA-256 by default,
  SHA-512, SHA-1, MD5 and CRC32 on request; each result shows as its file finishes;
  results are copied or saved as checksum files; a Verify page checks SFV, MD5, SHA-1,
  SHA-256 and SHA-512 files. A run has no limit on files, folder depth or checksum file
  size.
- **Open:** further algorithms and a hash plugin API (HASH-02).
- **Documents:** [checksums](developer/CHECKSUMS.md).
- **Verified:** `checksums`, `tools_checksum_algorithms`, `tools_checksum_manifests`,
  `tools_checksum_large_stop`, `tools_checksum_unlimited`, `journeys_find_to_checksums`;
  statuses in the [tools record](developer/acceptance/tools.md).

## Make File List

- **Windows and Linux:** Ctrl+M exports the selected entries' details with
  Salamander-compatible templates to the clipboard, the viewer or a UTF-8 file.
- **Open:** in a Network Neighborhood location it refuses with "Wait for the directory
  listing to finish.", which never becomes true there (FILELIST-01).
- **Documents:** [Make File List](MAKE_FILE_LIST.md),
  [Make File List service](developer/MAKE_FILE_LIST.md).
- **Verified:** `files_file_list`, `panels_file_list`; statuses in the
  [panels record](developer/acceptance/panels.md).

## Custom Commands, command line and editors

- **Windows and Linux:** Custom Commands on the toolbar and in the User menu run
  programs with argument templates, or scripts in PowerShell, cmd or a POSIX shell.
  The command line runs commands in the chosen shell and inserts names and paths
  (Ctrl+Enter). The terminal button opens a terminal in the panel's folder; Ctrl+click
  opens it as administrator (sudo on Linux). F4 opens the configured external editor,
  and Enter opens documents with their associated application.
- **Windows and Linux (0.5.1):** a Run directly command shows its output hidden (as
  before), in an output window (live standard output and error, exit code, Stop, Copy
  all, Save as…, Clear, Find, close when finished successfully) or in a console window
  that stays open; the literal argument rows reach the program unchanged in every mode.
  Settings schema 39. Verified on Windows; the Linux code's unit tests pass in hosted
  CI on Ubuntu 24.04, and no acceptance check has run it on Linux.
- **Open:** terminal sessions are not reused (TERM-01).
- **Documents:** [Custom Commands](developer/USER_COMMANDS.md),
  [Custom Command settings](developer/USER_COMMAND_SETTINGS.md).
- **Verified:** `options_custom_commands`, `options_editors`, `tools_user_menu`;
  statuses in the [Options record](developer/acceptance/options-tools.md) and the
  [tools record](developer/acceptance/tools.md). The terminal dry run is skipped where
  no supported terminal is installed.

## Options, keyboard and appearance

- **Windows and Linux:** Options pages for general settings, appearance (26 themes,
  density, fonts, the Scrolling glide slider: Off, Very short, Short, Default, Long or
  Very long, and the Scrolling speed slider: Very slow, Slow, Medium, Fast or Very fast
  wheel notches, both applied at once to panels, lists, dialogs and every viewer),
  filename colours, previews, archives, editors, comparators,
  Custom Commands, keyboard shortcuts and plugins, and the Manage Hot Paths window;
  export and import of the
  settings file; F1 keyboard help and the F10 menu; windows down to 320×200.
- **Open:** at 320×200 with touch density and a 32 px font the colour picker scrolls
  (OPTIONS-02); with the installed plugins, Hot Paths inside archives are checked only four
  archives deep in `viewers_archives` (added with the star, opened by number); the manager,
  menus, a restart and a changed archive are not (HOTPATH-02).
- **Documents:** [keyboard shortcuts](KEYBOARD_SHORTCUTS.md),
  [settings files](SETTINGS_FILES.md),
  [filename colours](developer/FILENAME_COLOR_OPTIONS.md),
  [dialog ergonomics](developer/DIALOG_ERGONOMICS.md),
  [theme provenance](THEME_PROVENANCE.md).
- **Verified:** `options`, `options_general`, `options_appearance`, `options_colors`,
  `options_palette`, `options_previews_plugins`, `options_archives`, `options_editors`,
  `options_comparators`, `options_hot_paths`, `options_keyboard`, `options_buttons`,
  `options_settings_file`, `options_themes`, `options_visual`, `shortcuts_catalog`,
  `shortcuts_footer`, `shortcuts_hints`, `journeys_options_restart`,
  `journeys_keyboard_only`, `help`, `about_box`, `tools_main_menu`; statuses in the
  [Options record](developer/acceptance/options-tools.md), the
  [tools record](developer/acceptance/tools.md) and the
  [UI polish record](developer/acceptance/ui-polish.md) (`ui_review`,
  `ui_polish_checks`, `ui_polish_dialogs`).

## Drive information

- **Windows and Linux:** Ctrl+F1 or Ctrl+L shows a volume's filesystem, root, label,
  capacity, storage kind and reported properties, queried in the background.
- **Documents:** [drive information](DRIVE_INFORMATION.md).
- **Verified:** `tools_help_drive_info`; statuses in the
  [tools record](developer/acceptance/tools.md).

## Plugin interfaces

- **Implemented:** the viewer C ABI 1.0 with plugin API and SDK 0.2.0; the archive
  worker API 0.5.0; the native filesystem plugin ABI 1.5 with API and SDK 0.2.0; the
  image viewer API 0.3.0 with SDK 0.4.0; the web viewer API and SDK 0.4.0; the
  thumbnail API 0.1.0 with SDK 0.2.0; the worker SDK 0.2.0. The text, source and
  Markdown viewers and the native and Network Neighborhood filesystems load as native
  libraries; the image and PDF viewers, thumbnailers, video playback and the archive
  worker run as restricted processes; the HTML viewer runs Chromium with its sandbox.
- **Open:** panels browse and read content through the native filesystem plugins,
  while watches, file operations, transfers and Find still use the host's platform
  services (VFS-01); SDK-01 to SDK-05, REL-01, PLUGIN-01 and PROC-01.
- **Documents:** the [developer guide](developer/README.md) and the API references it
  lists.
- **Verified:** each API workspace's tests and the plugin CI jobs, and the
  [viewer ABI conformance fixture](../plugins/viewer-conformance/README.md).

## Internal engine contracts

- **Implemented:** backend work runs off the UI thread with bounded queues,
  cancellation and owned cleanup. Each engine reserves one retirement domain for the
  owners its capacity table lists, and the test
  `lifetime_document_states_the_domain_capacity_and_its_breakdown` keeps that table
  equal to the code.
- **Open:** a quarantined clipboard or drop capture keeps its permits until exit
  (LIFE-02); a temporary folder on an unresponsive network filesystem can block report
  logs (SPOOL-01); on Linux a listed link into a stalled network mount holds its
  listing page (LINK-01).
- **Documents:** [engine lifetime](developer/ENGINE_LIFETIME.md),
  [security requirements](../SECURITY.md).
