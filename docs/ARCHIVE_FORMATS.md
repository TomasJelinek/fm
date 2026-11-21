# Archive formats and panel associations

Enter or a left double-click opens an enabled archive association as a folder only
when a trusted, compatible filesystem worker advertises its decoder. A small zipper
badge marks these files in Detailed, Brief and Thumbnails views, alongside the host icon.
The badge is a handler hint, not a claim that the contents have already been validated.
No file-content probing or archive indexing happens in the UI or during badge rendering.
Browsable archives use a small, muted corner zipper without an opaque background:
8 × 12 logical pixels in Detailed/Brief views and 10 × 17 in Thumbnails. The
associated file icon and selection remain visible, and row sizes do not change.
The fallback archive icon already includes a zipper and receives no duplicate overlay.

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

## Associations

Options → Archives has a format chooser for ZIP, 7-Zip, TAR, GZIP, XZ, Bzip2,
Zstandard and each supported compressed TAR family. Each format shows its built-in
association checkboxes and a separate custom-extension list with Add and Remove.
A saved override
changes panel activation and the badge immediately; it does not register an operating
system association. Missing settings retain the defaults below. Unchecked documents,
media, applications and installers open through the existing native Open action.
A checked association still needs its provider installed. Disabling automatic opening
does not invalidate a previously saved typed archive tab or change decoder support.

Overrides use the existing backend preferences store. Schema 13 stores a bounded
map of canonical lowercase extensions to booleans; unknown, repeated or malformed
entries reject the settings document rather than silently changing an association.
When no overrides exist, the writer retains the oldest schema needed by the other
saved settings. Changes made before startup settings finish loading take precedence
for the edited extension while preserving unrelated saved choices.

Custom extensions are shown as removable chips. Their removal cross is red,
using a darker shade in light themes and a lighter shade in dark themes.
Disabled actions retain the normal muted appearance.

Custom extensions accept one literal suffix at a time, with an optional leading
dot. Unicode case folding makes `.BACKUP`, `.backup` and equivalent Unicode case
spellings match; composed and decomposed Unicode remain distinct. Compound suffixes
such as `backup.tar` match before shorter suffixes. A disabled built-in compound
suffix blocks a shorter enabled match. Raw native filenames and paths are never
rewritten; valid Unicode suffixes also work after undecodable native basenames.
No UI callback opens files or probes content. Backend enumeration prepares the
folded suffix hint, and the worker still validates the actual archive bytes.

Assignments are globally unique, and every built-in suffix remains reserved even
when its checkbox is off. A conflict with a built-in offers **Show built-in checkbox**;
other conflicts name the assigned decoder. Invalid input retains the draft. Drafts
also survive switching formats and closing/reopening Options. Add is available
after initial settings loading; accepted additions/removals save immediately.
Suffixes permit Unicode, letters, digits, dots, `-`, `_` and `+`; empty components,
spaces, path separators, wildcards and command syntax are rejected. The collection
is bounded to 256 extensions, each at most 256 UTF-8 bytes after case folding.
These are settings limits and do not impose a native path-length limit.

Schema 20 stores custom suffixes with stable decoder identifiers. Strict decoding
rejects unknown decoders, duplicate/noncanonical suffixes and built-in collisions.
Earlier settings migrate with an empty custom list. New saved archive addresses
use address version 2 and retain the exact decoder, so removing a custom suffix
does not invalidate an existing archive tab or Hot Path. Legacy address version 1
continues using built-in suffix inference. Adding an extension cannot install,
trust or enable a missing decoder, and does not change OS associations.

## ZIP containers

These 51 suffixes share the ZIP decoder. Case is ignored for the ASCII suffix;
native Unicode and non-Unicode filenames remain lossless. Only regular files receive
a badge: a directory named `example.egg` stays an ordinary directory, and links never
do. Readable members of an archive whose names have an enabled association carry the
badge too, since Enter opens them as [archives inside the archive](#archives-inside-archives).

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

Office/OpenDocument documents and templates are deliberately **disabled by default**:
their normal activation should launch Word, Excel, PowerPoint, LibreOffice or the
user's chosen application. The same policy applies to books/comics, maps, image
projects, executable JAR/zipapps and installers/extensions. A user can explicitly
enable any of these containers for archive browsing. A suffix is never enough to
run package scripts, install code, decrypt content or guarantee that a container
variant uses a supported codec. In particular, some `.pyz` files are non-ZIP
application payloads; the ZIP worker rejects those instead of executing them.

## Other enabled archive formats

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

Compound `.tar.gz` is recognized before `.gz`. Provider discovery reports actual
supported families; installing only a TAR provider does not enable ZIP badges.

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
every later request through that session fails too. The panel then never lists
through that session again: it opens the archive once more at the requested folder
(Backspace, Enter on `..`, Refresh, sorting or a re-listed tree), and if that fails
as well it returns to the native folder that contains the archive, with the archive
under the cursor, and shows the error. A cancelled listing changes nothing.

ZIP support is currently Stored/Deflate without passwords; other codecs, encrypted
Office containers, DRM and corrupt archives may fail explicitly. The current worker
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
lease no longer depends on the enclosing mount's member cache or worker, and every read
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

The canonical host table is
[`archive_format.rs`](../crates/midnight-core/src/archive_format.rs).
Extension policy remains host-owned: the independent worker receives a decoder
family and captured source bytes, not UI settings or native paths.

Salamander reference reviewed: `src/dialogsp.cpp` (`CCfgPageArchivesAssoc`) keeps
panel archive associations separate from external file-opening associations;
`src/pack3.cpp` persists the panel extension list. Midnight Explorer uses the
requested per-extension checkboxes, format selection and custom Add/Remove controls
with native-path-safe Rust representation. No reference code or assets were copied.
