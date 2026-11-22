# Archive formats and panel associations

Enter or a left double-click on a file whose extension has an enabled archive
association opens the file as a folder, read by the installed archive worker. When no
installed worker supports its format, the panel says "Compatible archive filesystem
plugin is unavailable", with the plugin discovery diagnostics, and does not hand the
file to another application. Files without an enabled association open as usual.

A small zipper badge marks the files that open this way in the Detailed, Brief and
Thumbnails views. It is a muted corner zipper without an opaque background, 8 × 12
logical pixels in Detailed and Brief and 10 × 17 in Thumbnails, drawn over the system
icon; the icon and the selection stay visible and row sizes do not change. A file gets
the badge only when its association is enabled and an installed worker supports its
format. The badge is a hint from the file name, not a check of the contents: neither
the UI nor the badge reads file contents or indexes archives. When the system has no
icon for the file, the built-in archive icon is shown instead, without a badge.

## Opening and reading archives

Archive indexing and decoding run in the restricted background worker. ZIP and
7z open by indexing their metadata, without decompressing file payloads. A 7z
compressed header still needs decoding at Open. The first ZIP member read decodes
and checks that complete member; the first 7z read decodes only the requested
compression block and checks its member lengths and available CRCs before
returning any bytes. Damage to an unrelated 7z block is reported when that block is
read, not while browsing intact metadata. Listing is not a full integrity test of
the archive.

TAR indexes checked member ranges without reading file payloads. Compressed TAR and
standalone compressed streams first decode and validate their complete outer stream
into one private scratch file; members then reuse checked ranges of it. The worker
reads source bytes on demand through the host's revision-checked source broker and
streams selected members into private host-owned spool files, so the complete
archive never stays in worker memory; source reads are bounded to 256 KiB, and
indexes fetch only the ranges they parse. Verified siblings of the last decoded
solid 7z block do stay there, up to 128 MiB with later members first: members read
within that window need no further block decode, while a larger block or a read
outside the window decodes the whole block again. Each mount keeps
recently read members for repeated reads (at most 16 members within 512 MiB of
spool storage). Worker startup and indexing remain part of opening time. These
changes do not remove resource limits, source revision validation or sandbox
isolation. Copy-out still independently verifies destination bytes with SHA-256;
archive CRC checks do not replace that verification. The
[measured opening results](verification/2026-11-08-archive-opening-performance.md)
record the fixture sizes, cold-start limitation and regression coverage of that
earlier in-memory design.

Inside an archive the path bar shows the native folders that hold the archive as
separate parts, as on a native path, then the archive's name and its folders, each a
part of its own. A path too wide for the bar shows whole parts only: up to two
leading parts (the root first), `...` for the omitted middle and as many of the last
parts as fit. The archive's name (in an archive inside an archive, the innermost
one's) and the current folder stay whole as long as both fit: the leading parts give
way first, and in a still narrower bar the archive's name stands between two `...`
markers, the second hiding its deeper folders, followed by as many of the last
folders as fit. The current folder is shortened at its end only when it alone is too
wide. Clicking `...` edits the whole path. Clicking a native folder's part leaves the
archive for that folder, with the cursor on the folder or archive on the way back; a
part inside the archive lists that archive folder.

## Members and dates

- Members keep the names the archive records, raw and lossless. Members of a TAR or
  7z archive that share a name stay separate entries. ZIP records that the ZIP reader
  indexes under one name are listed once and cannot be read.
- A GZIP, XZ, Bzip2 or Zstandard file that holds no TAR shows one member named
  `content`: the decompressed stream. The file name stored in a GZIP header is not
  used.
- Links, special entries and unsafe names (absolute paths, `..` components,
  backslashes, colons) are listed but cannot be read or copied out; the status line
  of such a member says **Unreadable** and why.
- Dates: a ZIP member shows its NTFS or extended timestamp when the archive records
  one, and otherwise the recorded local time as it is, without a time zone
  conversion. TAR dates (including PAX times) and 7z dates are UTC and are shown in
  local time.

## Associations

**Options → Archives** sets which file names open as folders. It has a format chooser
(ZIP, 7-Zip, TAR, GZIP stream, TAR.GZ, XZ stream, TAR.XZ, Bzip2 stream, TAR.BZ2,
Zstandard stream and TAR.ZST), a line that says whether an installed worker supports
the chosen format, a **Filter file types** field, a checkbox for each built-in
extension of the format and the format's custom extensions.

- Changes apply at once to panel activation and badges. **OK** saves them with the
  other options and **Cancel** restores the previous choices. They never change the
  operating system's file associations.
- An unchecked extension opens in its usual application, as other files do. A checked
  one still needs an installed worker for its format: without one, the page says
  "Decoder unavailable" and keeps the choices.
- Turning an association off does not invalidate a saved archive tab or Hot Path, and
  does not change what the worker can decode.
- Extensions without a saved choice use the defaults listed below.

### Custom extensions

The **+** button opens a field: type a suffix, with or without its leading dot, and
press Enter to assign it to the chosen format; Escape cancels the edit. Each custom
extension shows as a chip whose red **×** removes it. Adding and removing wait until
the saved settings have loaded.

- A suffix is literal: letters (any Unicode letters), digits, dots, `-`, `_` and `+`.
  Empty parts, spaces, path separators, wildcards and command syntax are refused. At
  most 256 custom extensions are kept, each at most 256 UTF-8 bytes after case
  folding; these are settings limits, not path-length limits.
- Matching ignores letter case by Unicode case folding, so `.BACKUP` matches `.backup`;
  composed and decomposed Unicode stay different. Native file names are never
  rewritten, and a valid Unicode suffix also matches after a part of the native name
  that is not valid Unicode.
- The longest suffix decides: `backup.tar` matches before `tar`, and a built-in
  compound suffix that is turned off (such as `.tar.gz`) keeps a shorter one (`.gz`)
  from opening the file.
- Each suffix belongs to one format. A built-in extension cannot be added as a custom
  one, even while its checkbox is off; the page offers **Show built-in checkbox**,
  which selects that format and filters the list to the extension. A suffix that is
  already assigned names the format that has it; remove it there first.
- A refused suffix stays in the field, with the reason below it. Typed text also
  stays when you switch formats or close and reopen Options.
- Adding an extension does not install, trust or enable a missing worker. The worker
  still checks the actual file contents when it opens the archive.

### Saved choices

The choices are stored with the other preferences. Built-in choices (settings schema
13 and later) store only the extensions whose state differs from the default, as
lowercase extension and on or off. Custom extensions (schema 20 and later) store each
suffix with the identifier of its decoder. An unknown or repeated extension, an
unknown decoder, or a custom suffix that is not canonical or collides with a built-in
one makes the settings file invalid rather than changing an association silently. A
settings file without such choices keeps the oldest schema that its other settings
need. A checkbox changed before the saved settings have loaded keeps that change, and
the other saved choices load as stored.

Saved archive tabs and Hot Paths record the exact decoder (archive address version
2), so removing a custom suffix does not break them. Addresses saved before
(version 1) infer the decoder from the built-in suffix.

## ZIP containers

These 51 suffixes share the ZIP decoder. Their letter case is ignored; native
Unicode and non-Unicode file names stay lossless. Only regular files get the badge
and open as folders: a folder named `example.egg` stays a folder, and a link to an
archive opens like any other link. Readable members of an archive whose names have
an enabled association get the badge too, since Enter opens them as
[archives inside the archive](#archives-inside-archives).

| Purpose | Extensions | Default | Format reference |
| --- | --- | --- | --- |
| ZIP archive | `.zip` | Enabled | Standard ZIP |
| Python package | `.whl`, `.egg` | Enabled | [wheel](https://packaging.python.org/en/latest/specifications/binary-distribution-format/), [egg](https://setuptools.pypa.io/en/latest/deprecated/python_eggs.html) |
| Java deployment archive | `.war`, `.ear` | Enabled | [Oracle packaging specification](https://docs.oracle.com/cd/E19226-01/820-7627/bnaby/index.html) |
| Android development package | `.aab`, `.aar` | Enabled | [AAR](https://developer.android.com/studio/projects/android-library#aar-contents), [AAB parser](https://github.com/google/bundletool/blob/master/src/main/java/com/android/tools/build/bundletool/model/AppBundle.java) |
| NuGet package | `.nupkg`, `.snupkg` | Enabled | [NuGet packages](https://learn.microsoft.com/en-us/dotnet/standard/library-guidance/nuget) |
| Java application | `.jar` | Disabled | [JAR specification](https://docs.oracle.com/javase/8/docs/technotes/guides/jar/jar.html) |
| Python application | `.pyz`, `.pyzw` | Disabled | [PEP 441](https://peps.python.org/pep-0441/) |
| Android application installer | `.apk` | Disabled | [APK Analyzer](https://developer.android.com/studio/debug/apk-analyzer) |
| Visual Studio / VS Code extension | `.vsix` | Disabled | [VSIX](https://learn.microsoft.com/en-us/visualstudio/extensibility/visualstudio-extensibility/ship/vsix-projects) |
| Firefox extension | `.xpi` | Disabled | [Mozilla XPI packaging](https://extensionworkshop.com/documentation/publish/package-your-extension/) |
| E-book / comic | `.epub`, `.cbz` | Disabled | [EPUB](https://www.w3.org/TR/epub-33/), [calibre comic implementation](https://github.com/kovidgoyal/calibre/blob/master/src/calibre/ebooks/comic/input.py) |
| Google Earth map | `.kmz` | Disabled | [KMZ](https://developers.google.com/kml/documentation/kmzarchives) |
| Krita / OpenRaster image | `.kra`, `.ora` | Disabled | [KRA](https://docs.krita.org/en/general_concepts/file_formats/file_kra.html), [ORA](https://docs.krita.org/en/general_concepts/file_formats/file_ora.html) |
| Word document / template | `.docx`, `.docm`, `.dotx`, `.dotm` | Disabled | [Office formats](https://learn.microsoft.com/en-us/office/compatibility/office-file-format-reference) |
| Excel workbook / template / add-in | `.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.xlam`, `.xlsb` | Disabled | [Office formats](https://learn.microsoft.com/en-us/office/compatibility/office-file-format-reference) |
| PowerPoint presentation / template / add-in | `.pptx`, `.pptm`, `.potx`, `.potm`, `.ppsx`, `.ppsm`, `.ppam`, `.sldx`, `.sldm` | Disabled | [Office formats](https://learn.microsoft.com/en-us/office/compatibility/office-file-format-reference) |
| OpenDocument document / template | `.odt`, `.ott`, `.odm`, `.otm`, `.ods`, `.ots`, `.odg`, `.otg`, `.odp`, `.otp`, `.odf`, `.odb` | Disabled | [LibreOffice XML formats](https://help.libreoffice.org/latest/en-US/text/shared/00/00000021.html) |

Office and OpenDocument documents and templates are deliberately **disabled by
default**: Enter should open them in the user's office application. The same applies
to e-books and comics, maps, image projects, executable JAR and Python applications,
and installers and extensions. Any of them can be enabled for archive browsing. A
suffix never runs package scripts, installs code or decrypts content, and does not
guarantee that the container uses a supported compression method. Some `.pyz` files
are not ZIP files at all; the worker reports "This file is not a valid ZIP archive."
and nothing is run.

## Other archive formats

These formats are enabled by default:

| Decoder | Default extensions |
| --- | --- |
| 7-Zip | `.7z` |
| TAR | `.tar` |
| GZIP single stream | `.gz`, `.gzip` |
| Gzipped TAR | `.tar.gz`, `.tgz` |
| XZ single stream | `.xz` |
| XZ-compressed TAR | `.tar.xz`, `.txz` |
| Bzip2 single stream | `.bz2` |
| Bzip2-compressed TAR | `.tar.bz2`, `.tbz2`, `.tbz` |
| Zstandard single stream | `.zst` |
| Zstandard-compressed TAR | `.tar.zst`, `.tzst` |

Compound `.tar.gz` is recognized before `.gz`. The installed worker's manifest lists
the formats it supports, and only those get badges and open as folders.

The worker decodes these compression methods:

- ZIP: Stored and Deflate, also when encrypted (see
  [password-protected archives](#password-protected-archives)).
- 7z: Copy, LZMA and LZMA2, the BCJ filters (x86, BCJ2, ARM, ARM64, ARM Thumb,
  PowerPC, IA-64, SPARC and RISC-V) and Delta, in solid and non-solid archives, also
  with AES-256 encryption.
- GZIP (including several concatenated members), XZ, Bzip2 and Zstandard, as single
  streams and around a TAR. Zstandard skippable frames are skipped, and frame
  checksums are verified when present.

A member stored with another method, for example ZIP Deflate64, BZIP2 or Implode, or
7z PPMd, is listed with its size but cannot be read, and its status line names the
method; the rest of the archive stays readable. Members of split (multi-volume)
archives cannot be read either. RAR is not supported.

## Current operation limits

The independent archive worker currently supports browsing, sorting/filtering,
selection and F3 member reads. **F5 copy out / extraction is implemented** through
the bounded background transfer queue, with progress, conflict handling, safe
destination traversal and independent SHA-256 destination readback. See the
[archive-copy evidence](verification/2026-11-07-archive-copy.md).
Copying into an archive and moving archive members are not implemented.

Archive Hot Paths persist typed container/provider/member locations and the exact
decoder, and display `archive.zip@path` with native separators. The display string
is never parsed to reconstruct an operation target. Duplicate detection compares
the archive location independently of its captured source revision. Re-adding a
location refreshes that revision while retaining its shortcut slot. If the archive
has changed since capture, navigation reports that change; reopen the archive and
add the Hot Path again to capture its new revision. A change of the archive file's
change time alone, which endpoint protection makes for example by adding an alternate
data stream, is not a change: an open archive reads on and a Hot Path or saved tab
still opens it, as long as its identity, length and modification time are the same
([change-time drift](developer/CONTENT_READS.md#change-time-drift-view-05)). A Copy
out of an archive that did change offers Skip but no Retry, which could not succeed.
Missing providers remain an
explicit unavailable location, with native file operations disabled. Core/codec
and presenter tests cover this implementation; final integrated UI qualification
is tracked in the [current extension](superpowers/specs/2026-11-07-panel-viewer-followups.md#archive-hot-paths).

A failed listing in an archive panel can mean that the archive's worker was
stopped, for example by its 20-second idle watchdog while a viewer read a member;
every later request through that session fails too. A Copy out of that archive then
offers Skip, Skip all and Cancel, not Retry, for its remaining members: open the
archive again and copy the rest anew. The panel then never lists
through that session again: it opens the archive once more at the requested folder
(Backspace, Enter on `..`, Refresh, sorting or a re-listed tree), and if that fails
as well it returns to the native folder that contains the archive, with the archive
under the cursor, and shows the error. A cancelled listing changes nothing.

Panels, tabs and viewers keep at most two archives opened from native files open at
once; tabs and viewers that show the same archive share it. A tab that leaves its
archive for another one (a Hot Path or saved tab, Find's Go to file, or Enter on an
archive right after leaving one) gives up its archive, and the new archive takes its
place as soon as the old one has closed, normally at once and never later than after
10 seconds: a tab's own archive never blocks its next one. The tab keeps the old
archive's folders in its history by their addresses, so going back to one opens that
archive again. An archive that another tab, the other panel or a viewer still shows
stays open and counts as before, so the new one is refused at once with "Two archives
are already open; close one of them first"; one that a running copy still reads from
also stays open, and the new archive is refused after those 10 seconds.

ZIP support is Stored/Deflate, including password-protected members (below); other
codecs, encrypted Office containers, DRM and corrupt archives fail explicitly, and the
panel names the reason. The current worker
has no artificial source, member or expansion-ratio byte ceiling: available spool
storage limits decoded content. Nothing limits how many members an archive has, how
many a folder lists or how long their names are in total: the archive's index lives
in its worker, whose 1 GiB of memory holds about nine million TAR, two million 7z or
one and a half million ZIP members, and panels, F5 copy-out, folder sizes and Find
all take a folder of any size; F5 also copies a selection of any size. A panel
showing an archive folder holds about 640 bytes per member in the application (about
610 MiB for a million members). An
archive that needs more than its worker has says that the worker ran out of memory;
the worker SDK grants no worker more than 1 GiB, so this is the remaining limit
(protocol-1 workers keep their legacy limits, including a 64 MiB source). A source
that cannot be read, such as a dropped share, is reported as such, never as an
invalid archive. RAR is unavailable; archives inside archives are described below.
See the [filesystem API](developer/FILESYSTEM_API.md) and the
[archive worker](../plugins/midnight-fs-archive/README.md) for exact limits and the
plugin contract.

## Password-protected archives

Encrypted members are listed like any other, with the length the archive records, and
a small padlock beside their icon (" · Encrypted" for screen readers). The status line
of a focused encrypted member adds **Encrypted** with its method: ZipCrypto (the
traditional ZIP encryption), AES-128, AES-192 or AES-256 (WinZip AES) or 7-Zip
AES-256. A member that cannot be read says why, in its status line (**Unreadable**) and
when it is opened: for example "old.bin is compressed with Implode, which the archive
plugin cannot decompress.", a link, an unsafe name, several records with one name, or
PKWARE strong encryption, which is not supported.

One password opens an archive, and it is asked once: in a small dialog, when an
action first needs it. That is F3 or Alt+F3 on an encrypted member, Enter on an
encrypted archive inside the archive, F5 of a selection that holds encrypted members
(or folders that contain some), or opening a 7z archive whose file names are
encrypted. Type the password (Show password reveals it) and press Enter or OK; a wrong
password keeps the dialog open with "Wrong password — try again." Escape, Cancel or
the close button leave the archive locked and close the dialog without a further
message: F3 and Enter open nothing, and the keyboard is back in the file list with the
cursor on the same row. A copy goes on and skips each protected member, naming that
reason in its results, without asking again. The next action that needs the password
asks for it again. If the archive plugin cannot decrypt the members, or the archive
fails while the password is checked, the dialog closes and a message says why. Find's
search inside archives never asks: it skips protected members with that reason, and
previews of protected members appear once the archive has been unlocked in its panel.

Once accepted, the password is remembered until the archive is closed, and F3, Enter
and F5 use it without asking. The archive is closed when nothing uses it any more:
every panel tab that showed it has left it or been closed (a tab that is not in front
keeps its archive open), every viewer of one of its members is closed and every copy
from it has finished. Exiting the application closes every archive. Opening the
archive again after it was closed, or opening the same file separately in the other
panel, starts a new session, which asks again when it needs the password. An archive
inside an archive asks for its own password, separately from the archive that
contains it, and keeps it by the same rule. A panel that has left an archive inside
an archive keeps that archive open for Back and Forward only until the panel opens
another archive inside an archive or leaves the enclosing archive.

The password is kept in memory only: it is never saved in settings, history, logs,
error details or the clipboard, and it goes away when the archive closes, because the
archive's worker process, the only place that keeps it, then ends. Only that sandboxed
worker decrypts, and every member is checked (CRC-32, and for WinZip AES its
authentication code) before any of its bytes is shown or copied: a damaged member or a
password that only seemed right fails with an error instead of producing wrong data.
ZIP passwords are taken as UTF-8 text, so a non-ASCII ZipCrypto password that another
program stored in an old code page may not match. Members with a different password
than the archive's first encrypted members are reported as needing another password.

## Archives inside archives

Enter or a left double-click on a readable archive member whose name has an enabled
archive association opens that member as an archive in the same panel: a ZIP inside a
ZIP, a TAR or gzipped TAR inside a ZIP, or any supported format inside any other. The
path bar names the whole path, for example `/data/outer.zip/folder/inner.zip/sub`, and
its parts open their folders, including the folders of the enclosing archives.
Backspace and `..` go up through the levels: from the root of an inner archive to the
enclosing folder with the inner archive under the cursor, and from the outermost root
to the native folder with the archive file under the cursor. Back and forward history
covers every level. Enter on an inner archive that the panel still remembers lists it
through its existing session without staging it again; opening another archive inside
an archive first ends the sessions of inner archives the panel has left, whose folders
then leave its history. F3 views the members of an inner archive and F5 copies them out
with the usual SHA-256 destination readback; folder sizes and member previews go
through the inner archive's session as in any archive.

Opening stages the member through the enclosing archive's session: its worker decodes
and verifies the whole member (lengths and CRCs, as for F3), and the host keeps the
result as a private lease, in memory when it fits the 16 MiB stage limit and the
shared 128 MiB staging budget, and otherwise in an anonymous private spool file under
the same free-space reserve as every member. Such a member's length is reserved on the
spool volume before anything is staged: when it would not fit above the volume's
reserve, beside what other archive reads in progress still need, opening it is refused
at once, saying how much space it needs and how much there is
(`This archive member needs 50.0 GiB of archive spool space, but the spool volume has
only 20.0 GiB free above its reserve…`), and the panel stays in the enclosing folder,
whose archive stays open (a protocol-2 worker cannot be refused without ending, as on
any staging failure). F3 and F5 on such a member are refused the same way. The inner
archive gets a restricted worker of its own, which reads that lease through the
ordinary bounded source broker. The
lease does not depend on the enclosing mount's member cache or worker, and every read
of the inner archive revalidates the enclosing chain down to the native file: a changed
outer file makes the inner archive fail instead of serving old bytes. While a large
member is staged, the status bar shows its progress (`Opening inner.zip: 12 of 340
MiB…`), at most five times a second. Backspace or any other navigation of the panel
cancels the staging between bounded steps; the enclosing archive's worker keeps
serving its other users.

Limits:

- At most 4 archive levels, counting the archive opened from a native file. Enter on
  an archive at the deepest level explains the limit; copy it out with F5 to open it.
- At most 3 mounts of archives inside archives at once, besides the two archives
  opened from native files and the two that Find's Search inside archives may hold
  (each search opens one archive at a time), each worker with the usual 1 GiB
  memory limit.
- No additional byte, expansion-ratio or member-count ceiling, consistent with the
  limits above: decoded content is bounded by the spool volume's free-space reserve
  (checked before a member is staged, as above), each level's index by its own worker's
  memory, and decoder windows and dictionaries
  by their usual bounds. Each level opens only on an explicit Enter, so an archive
  that contains itself is never unpacked recursively.
- Member names stay opaque provider identities. No worker receives a path, the staged
  member has no pathname, and copying out uses the existing safe destination traversal.

Errors name the member's place inside the enclosing archives. A member that is not a
valid archive, such as a text file named `.zip`, reports
`This file is not a valid ZIP archive.`; a corrupt inner archive reports the decoder's
reason. The panel then stays in the enclosing folder. When the worker of an inner
archive stops, for example by its idle watchdog, the panel does not reopen it: it
returns to the enclosing folder with the inner archive under the cursor, and says why.

An archive inside an archive is opened for the session only. Saved tabs and Hot Paths
keep the folder of the outermost archive that leads to it, and restoring them reopens
that folder. Persistent preview caching of its members applies only while every
enclosing provider grants stable member IDs. Inline folder branches do not expand
archives: Enter opens an archive inside an archive from the panel's own folder, while
archive members shown in expanded branches below it do not open.

The canonical extension table is
[`archive_format.rs`](../crates/midnight-core/src/archive_format.rs). The host owns
the extension policy: the independent worker receives a decoder family and the
captured source bytes, never UI settings or native paths. The worker's build, tests
and protocol are described in the
[archive worker development guide](archive-filesystem-development.md).

Behavioral reference: Salamander keeps panel archive associations
(`src/dialogsp.cpp`, `CCfgPageArchivesAssoc`) separate from the associations that open
files in other applications, and saves the panel extension list (`src/pack3.cpp`). No
reference code or assets were copied.
