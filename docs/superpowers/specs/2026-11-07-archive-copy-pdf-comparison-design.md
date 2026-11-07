# Faster browsing, archive extraction and document comparison

This extends the existing viewer, filesystem and copy designs. Requested by the
user on 2026-11-07. Independent plugins remain separate Cargo projects with their
own tests and depend only on public APIs. Platform implementations stay separate.

## Decisions and alternatives

- Profile listing and archive entry before optimizing. Archive Open currently
  decodes every member; ZIP should index metadata first and decode on demand.
  Preserve source identity validation, CRC checks, parser sandbox and limits.
- Copy out through the existing background queue and verified transfer service,
  rather than introducing a second unsafe extraction path. Capture virtual
  source identities and the destination before confirmation. Use SHA-256 over
  source bytes and independently reread the completed destination. Publication
  remains conditional and never silently replaces an existing file. Cancellation
  removes only owned staging files; completed copies remain and source is intact.
- Add popular XZ, Bzip2 and Zstandard streams and corresponding TAR containers
  after pinned dependency/license review. RAR and obscure formats are not added.
- Prefer a pure Rust PDF worker (`hayro`, subject to pinned license verification)
  over distributing platform-specific PDFium binaries. Extend the raster-viewer
  protocol additively for page metadata and page viewports; old image messages
  remain compatible. PDF is a separately installed plugin, with no JavaScript,
  URL launching or unrestricted file access. Unsupported/encrypted documents
  produce a readable error. PDF fidelity limitations are documented.
- Ctrl+Shift+C compares two selected regular files in the active panel; otherwise
  one selected/focused regular file from each panel. Ambiguous selection is an
  error. Image pairs use the existing sandbox decoder, with Side-by-Side, Swipe
  and Onion Skin modes. Other supported text pairs use a bounded, cancellable
  line diff. Both inputs are captured and validated, not reconstructed from labels.
- Existing filename color rules take precedence over migration defaults. New or
  legacy preferences receive source/executable/image/video/document defaults;
  explicit empty rules are persisted so removing defaults survives restart.
- Drive toolbar icons grow to 20 px (mouse) and 26 px (touch), keeping current
  button, row and toolbar dimensions and the space before drive captions.
- The main window has an exact 320×200 logical minimum regardless of density or
  font. Preferred size remains 1340×820. Compact presentation uses the active
  pane plus an other-pane rail, a path field, file list and command editor.
  Auxiliary panel chrome returns when space permits. Function keys pan
  horizontally while Options/Jobs remain visible; held Ctrl slots keep captions,
  including visibly disabled unassigned keys. Real target capabilities govern
  file-action availability, with backend guards still authoritative.

## Review and validation

Test stale identities, archive traversal/duplicate entries, cancellation and
  checksum failure, malformed PDF/archive/media inputs, mismatched image sizes,
  ambiguous comparison selection, Unicode text, deleted color presets and icon
  clipping. Keep all generated evidence under `_build`. Compare measurements on
  identical fixtures; do not infer real-world gains from asymptotic improvements.

Custom commands are a design deliverable in this request; their full UI/runtime
implementation is tracked separately in the User Commands design.
