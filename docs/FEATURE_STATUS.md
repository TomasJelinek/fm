# Feature status for 0.1.0

This is an early preview. A design document is not evidence that a feature is available.

| Feature | Current implementation | Remaining work / design |
| --- | --- | --- |
| Native panels | Windows and Linux browsing, tabs/bookmarks, multiselection, locale sorting, watches, selection-aware directory sizes, three view modes, themes and configurable shortcuts | macOS adapter remains pending |
| Media thumbnails | Installed independent image/video workers, private persistent cache, bounded background decoding and progressive animation of visible selected videos; Options → Previews controls enablement and cache clearing | Requires trusted installed helpers. Images: PNG/JPEG/GIF/WebP/BMP/ICO/TIFF. Initial video codec/container set is limited; no MP4/H.264 promise. Network/unclassified storage requires opt-in; archive-member previews and a full image viewer remain pending. See [worker SDK](developer/WORKER_SDK.md) |
| Panel search/filter | Typing searches filename prefixes; optional quick-search filtering; wildcard, regex and typed panel filters | Separate recursive Find dialog is pending |
| Internal viewers and F4 | Dynamically loaded basic text/hex, source highlighting and Markdown packages; separate windows, scroll/search/copy, captured-order Space navigation; configurable external editor | Requires installed, enabled viewer packages in a trusted installation. Selection/copy is bounded to the loaded page; source highlighting covers the published catalogue, with plain-text fallback. See [viewer implementation](verification/viewer-implementation.md) |
| Copy | Ctrl+C/Ctrl+V and drag/drop use a bounded backend queue, progress/cancellation and independent SHA-256 destination readback | Full F5 dialog, replacement/merge, links and complete metadata preservation remain pending |
| Move | Not implemented; copy keeps the sources | F6/Cut and deletion only after every destination is verified remain pending |
| Find files | Not implemented as a separate dialog | [Accepted scope](superpowers/plans/2026-11-05-follow-up-stages.md): modeless, recursive name/content search, streamed bounded results, progress/cancellation; detailed design and Salamander source review remain pending |
| Native mutations | Rename, empty-file creation, recursive F7, confirmed Trash/permanent deletion, attributes/timestamps/case changes | Capability limits and per-item failures remain explicit |
| Directory comparison | Ctrl+F10 comparison is implemented | Not a replacement for a recursive synchronization tool |
| Checksums | Design only; no checksum dialog/shortcut is implemented | [Ctrl+F8 design](superpowers/specs/2026-11-06-checksums-design.md): MD5/SHA-1 compatibility, SHA-256 default and SHA-512, background results and manifest export. Copy verification remains SHA-256 |
| Archive providers | Installed independent workers browse ZIP/7z/GZIP/TAR/TAR.GZ read-only, with parent navigation, sorting/filtering, selection, persistent provider tabs and F3 content leases | Bounded mounts: 64 MiB source/member and 128 MiB decoded total. No writing, extraction, passwords, nested mounts, RAR or external member launch. Missing/changed providers retain unavailable saved tabs. See [filesystem API](developer/FILESYSTEM_API.md) |
| Native and network providers | Native mounted filesystems/paths work through built-in OS adapters | Network discovery plugins, native-provider plugin migration and inline directory branches remain pending |

The [follow-up stages](superpowers/plans/2026-11-05-follow-up-stages.md) preserve
the uncompleted requests. Native copy currently runs serially with a 256 KiB
buffer. Future concurrent copying is permitted only when both source and
destination are positively classified as local SSD storage. HDD, network and
unknown storage retain serial copying. Details are in the repository's security
and architecture documents.

The release archives include this status file; relative design links are intended
for the repository version at https://github.com/TomasJelinek/fm.
