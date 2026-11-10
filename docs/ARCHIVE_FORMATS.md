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
compressed header still needs decoding at Open. The first ZIP member read checks
its entire decoded content; the first 7z read checks the entire containing solid
block before returning any bytes. Completed blocks are retained for later reads
within the same mount. Damage to an unrelated 7z block is reported when that
block is read, not while browsing intact metadata. Listing is not a full integrity
test of the archive.

TAR keeps one backing buffer and indexes validated member ranges rather than
copying every file into a second buffer. Compressed TAR must still decode its
outer sequential stream before indexing; the decoded backing buffer is reused.
The host currently transfers the complete compressed source into the worker, so
source I/O and first worker startup remain part of opening time. These changes
do not remove resource limits, source revision validation or sandbox isolation.
Copy-out still independently verifies destination bytes with SHA-256; archive
CRC checks do not replace that verification.
See the [measured opening results](verification/2026-11-08-archive-opening-performance.md)
for the fixture sizes, cold-start limitation and regression coverage.

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
native Unicode and non-Unicode filenames remain lossless. Only regular native files
receive a badge: a directory named `example.egg` stays an ordinary directory, and
links/nested virtual archives do not promise unsupported mounting.

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
add the Hot Path again to capture its new revision. Missing providers remain an
explicit unavailable location, with native file operations disabled. Core/codec
and presenter tests cover this implementation; final integrated UI qualification
is tracked in the [current extension](superpowers/specs/2026-11-07-panel-viewer-followups.md#archive-hot-paths).

ZIP support is currently Stored/Deflate without passwords; other codecs, encrypted
Office containers, DRM and corrupt archives may fail explicitly. Source/member
limits are 64 MiB each and the decoded mount total is 128 MiB. Nested mounts and
RAR are unavailable. See [filesystem API](developer/FILESYSTEM_API.md) for exact
limits and the plugin contract.

The canonical host table is
[`archive_format.rs`](../crates/midnight-core/src/archive_format.rs).
Extension policy remains host-owned: the independent worker receives a decoder
family and captured source bytes, not UI settings or native paths.

Salamander reference reviewed: `src/dialogsp.cpp` (`CCfgPageArchivesAssoc`) keeps
panel archive associations separate from external file-opening associations;
`src/pack3.cpp` persists the panel extension list. Midnight Explorer uses the
requested per-extension checkboxes, format selection and custom Add/Remove controls
with native-path-safe Rust representation. No reference code or assets were copied.
