# Faster browsing, archive extraction and document comparison

This extends the existing viewer, filesystem and copy designs. Requested by the
user on 2026-11-07. Independent plugins remain separate Cargo projects with their
own tests and depend only on public APIs. Platform implementations stay separate.

## Decisions and alternatives

- Profile listing and archive entry before optimizing. ZIP indexes metadata and
  decodes a member only when it is read. Extend that behavior to 7z: index the
  header at Open, then decode and verify the requested solid block in full before
  returning any member bytes. Cache only complete, verified blocks; duplicate
  names retain independent ordinal identities. TAR indexes checked byte ranges
  in its immutable backing buffer instead of copying every member's payload.
  Compressed TAR still needs stream decompression to reach successive headers,
  but does not copy those decoded payloads again. Preserve source identity
  validation, CRC checks, parser sandbox and all declared/actual size limits.
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
  Page navigation preserves the current manual zoom/100% scale exactly, including
  mixed page sizes. Fit stays selected and adapts to each page; navigation resets
  the scroll origin to the start of the page without changing zoom preferences.
- Ctrl+Shift+C compares two selected regular files in the active panel; otherwise
  it uses the active panel's single selected file or focused file. One explicit
  opposite selection takes priority; with none, a backend worker finds the unique
  exact native basename in the opposite panel's captured visible listing,
  independently of its cursor. Missing, hidden, non-file or ambiguous matches
  open a File A / File B form, preserving any unambiguous native file in its
  original left/right slot. An empty, loading, parent, archive or ambiguous panel
  also offers manual choice; it never supplies an arbitrary fallback cursor.
  Each field accepts an absolute native path or Browse using the system picker.
  Compare/Enter submits explicitly; invalid inputs remain editable. Cancel,
  Escape and native close dismiss the comparator and cancel pending picking.
  The form lives inside the resizable native comparator window, with scrollable
  fields and reachable buttons at 320×200. Its toolbar action is available even
  without a panel target. Picker, validation, identity capture and file reads
  stay on the bounded backend; native paths survive display conversion unchanged.
  Native case/Unicode normalization is not guessed; select the
  opposite file explicitly to compare different names. Image pairs use the
  existing sandbox decoder, with Side-by-Side, Swipe
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
