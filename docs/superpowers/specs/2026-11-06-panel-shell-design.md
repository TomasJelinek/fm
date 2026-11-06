# Panel interaction, system menus and clipboard transfers

This user-requested extension takes priority over the unfinished viewer stage;
the viewer plan remains pending. Product name and binary stay Midnight Explorer
and `me.exe`/`me`. Existing safety, Unicode, native path, worker and plugin boundaries apply.

## Interaction

- Ctrl+Enter appends the focused active-panel filename using the selected shell's
  literal-argument rules, then focuses the bottom command edit. It never executes it.
- Plain left double-click reliably opens a folder/file in all three panel modes,
  including after the first click's selection acknowledgement or metadata refresh.
  A recycled row, a changed target or modified click must never open another item.
- Right-click on a selected file/folder preserves the selected group; on an
  unselected entry it targets that entry. Drive controls target the drive root.
  Windows uses the real Shell context menu, including extension submenus. Shell
  work belongs to a backend STA with a native owner/message loop. Paths remain
  Unicode/native and are never transformed into shell command text.
- Ctrl+C publishes the active selection, or focused item if selection is empty,
  as a native file clipboard payload. Ctrl+V captures the active destination and
  submits background work. Text fields retain their ordinary copy/paste behavior.
  Native-menu copy/paste actions reuse this integration. App copying must verify
  independently reread target bytes against a cryptographic source checksum;
  clipboard support must not silently delegate unverified app copies to Explorer.

## Presentation

- File-only selection shows its aggregate known byte size, formatted by system
  locale. Unknown metadata or overflow must not be represented as an exact total.
  Aggregation is on backend workers; selection feedback remains immediate.
- Each panel status bar reserves right-aligned space for available drive capacity
  in readable units. Queries remain asynchronous and cannot overwrite a newer path.
- A sort dropdown beside View offers name, size, modified time and extension,
  plus direction, in every panel mode; it uses the existing sort state and rules.
- Drive choices are grouped with nonselectable separators: drives, special paths,
  saved Hot Paths. Keyboard and pointer selection skip separators.
- The Hot Paths star adds the current folder (implementation choice); its adjacent
  arrow opens saved paths. Below a separator, Add path and Manage remain available.
  Existing bookmark identity, persistence and numbered shortcuts are reused.
- Operation dialogs use existing native/fallback semantic icons, clear spacing,
  bounded scrolling and visible buttons at supported sizes/densities. Deletion
  has red emphasis and distinguishes Trash from irreversible permanent deletion.
  Copy/move and overwrite dialogs follow the same visual family when enabled.

## Implementation choices and safety

The requested real system menu is preferred over an imitation menu on Windows.
Linux desktop environments have no single Windows-equivalent Shell menu contract;
their implementation must use supported desktop services and clearly describe any
capability difference. Windows/Linux code and tests stay in separate files.

Use the existing mutation queue and immutable source/destination captures for
clipboard transfers. No new UI I/O or private shell copy path. Preserve the
existing verified-copy and conditional-overwrite design: unsupported guarantees
are errors, never silent truncation, target deletion or source removal. Resource
limits cap work/buffers, not native path length. Network/unknown storage remains
conservative. Copy/move conflicts contain fresh source/target size and time.

All user-requested dialogs and controls receive real event/layout tests. Native
Windows menus need real-window smoke testing in addition to backend tests.
Consult Salamander implementation without importing its source. Update design,
keyboard help and verification evidence, then independently review the complete
change set and fix material findings before delivery.

## Implemented transfer boundary

The current copy service uses SHA-256, bounded 256KiB worker steps and an independent
full target reread. Sources remain intact. File and directory identities are pinned
when the queued job starts; captured paths cannot change with panel navigation.
Copy and mutation queues each accept 32 jobs and share one mutation gate; FIFO is
per queue, with cross-queue order determined by gate acquisition. Copy stays serial
on every storage class. Future parallel Copy/Move requires both source and
destination to be positively identified as local SSDs and a real shared physical
storage scheduler. HDD, network, unknown or failed detection at either endpoint
keeps the transfer serial, even when the other endpoint is an SSD.

Existing target names create versioned conflicts: Skip, Skip all, copy under another
name, Cancel, or Decide later. Jobs reopens a deferred conflict without blocking
panel interaction. No ordinary rename is advertised as conditional overwrite, and
no recursive source deletion is reused as Move. Windows and Linux stage/publish via
their retained native handles; Linux requires O_TMPFILE-capable destinations and
rejects unsafe shared parents for creating directories. Full guarantees and current
limits are listed in [SECURITY.md](../../../SECURITY.md).

Native Windows menus follow Salamander's canonical-verb interception, including
paste **inside** a sole clicked folder. Selected groups retain their captured paths;
stale destructive menu results cannot retarget a changed panel. Linux uses supported
application actions. X11/XWayland file clipboard owns and serves bounded transfers;
pure Wayland file-copy ownership is explicitly unavailable pending a bounded writer.

All application bindings become configurable according to the
[keyboard design](2026-11-06-configurable-shortcuts-design.md).
