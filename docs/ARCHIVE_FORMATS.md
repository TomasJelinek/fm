# Archive formats and panel associations

Enter or a left double-click opens an enabled archive association as a folder only
when a trusted, compatible filesystem worker advertises its decoder. An entrance-arrow
badge marks these files in Detailed, Brief and Thumbnails views, alongside the host icon.
The badge is a handler hint, not a claim that the contents have already been validated.
No file-content probing or archive indexing happens in the UI or during badge rendering.
Browsable archives use a small, muted corner chevron without an opaque background:
7 × 8 logical pixels in Detailed/Brief views and 10 × 10 in Thumbnails. The
associated file icon and selection remain visible, and row sizes do not change.

Options → Archives lists every known association with a checkbox. A saved override
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

Compound `.tar.gz` is recognized before `.gz`. Provider discovery reports actual
supported families; installing only a TAR provider does not enable ZIP badges.

## Current operation limits

The independent archive worker currently supports browsing, sorting/filtering,
selection and F3 member reads. **F5 copy out / extraction, copy into an archive and
Move are not implemented.** Native F5 and clipboard transfer queues remain separate
from archive member reads. Future extraction must use the same bounded queue,
progress, conflict policy, safe destination traversal and verified publication.

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
requested per-extension checkboxes and native-path-safe Rust representation.
