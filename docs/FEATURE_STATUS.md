# Feature status for 0.1.0

This is an early preview. A design document is not evidence that a feature is available.

| Feature | Current implementation | Remaining work / design |
| --- | --- | --- |
| Native panels | Windows and Linux browsing, tabs/bookmarks, multiselection, locale sorting, watches, directory sizes, three view modes, themes and configurable shortcuts | Image/video thumbnails and macOS adapter remain pending |
| Panel search/filter | Typing searches filename prefixes; optional quick-search filtering; wildcard, regex and typed panel filters | Separate recursive Find dialog is pending |
| Internal viewers and F4 | Public API/SDK groundwork and viewer-rendering support exist | Dynamic loading, complete plain text/binary, source highlighting and Markdown viewers, and configured external editor are pending; see [viewer design](superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md) and [implementation plan](superpowers/plans/2026-11-05-viewer-plugins.md) |
| Copy | Ctrl+C/Ctrl+V and drag/drop use a bounded backend queue, progress/cancellation and independent SHA-256 destination readback | Full F5 dialog, replacement/merge, links and complete metadata preservation remain pending |
| Move | Not implemented; copy keeps the sources | F6/Cut and deletion only after every destination is verified remain pending |
| Find files | Not implemented as a separate dialog | [Accepted scope](superpowers/plans/2026-11-05-follow-up-stages.md): modeless, recursive name/content search, streamed bounded results, progress/cancellation; detailed design and Salamander source review remain pending |
| Native mutations | Rename, empty-file creation, recursive F7, confirmed Trash/permanent deletion, attributes/timestamps/case changes | Capability limits and per-item failures remain explicit |
| Directory comparison | Ctrl+F10 comparison is implemented | Not a replacement for a recursive synchronization tool |
| Archive and network providers | Native mounted filesystems/paths work | Archive browsing (ZIP/7z/GZIP/TAR/TAR.GZ), network discovery plugins, native-provider plugin migration and inline directory branches remain pending |

The [follow-up stages](superpowers/plans/2026-11-05-follow-up-stages.md) preserve
the uncompleted requests. Native copy currently runs serially with a 256 KiB
buffer. Future concurrent copying is permitted only when both source and
destination are positively classified as local SSD storage. HDD, network and
unknown storage retain serial copying. Details are in the repository's security
and architecture documents.

The release archives include this status file; relative design links are intended
for the repository version at https://github.com/TomasJelinek/fm.
