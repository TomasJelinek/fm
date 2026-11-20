# Comparator identity contract D

Frozen public names in `midnight_compare_engine`:

```rust
pub enum IdentityVerdict {
    Pending,
    IdenticalFiles,
    IdenticalText,
    Different,
}

pub struct Frame {
    pub comparison_generation: u64,
    pub identity: IdentityVerdict,
    // Existing presentation fields are unchanged.
}
```

`comparison_generation` identifies the exact captured pair and attempt. It is
nonzero on published frames and increases using checked arithmetic for a new
session, accepted pair replacement or retry, including a retry of the same paths.
Resize, scrolling, zoom and image presentation mode keep the generation.
`geometry_revision` continues to govern geometry independently.

The UI reads `identity` directly. It must not infer identity from `status`,
accessible text, image dimensions, pixels or the visible text page. A new session,
accepted Compare/retry, `Status` or `ChooseFiles` clears any displayed verdict to
Pending. An output from a retired session cannot update the active session. Within
a session, frames older than the active comparison generation are stale.

- Pending: no current successful proof, including pending work, errors or cancellation.
- IdenticalFiles: every raw byte in both entire inputs was compared equal through
  the captured sources, with source identity/revision validation.
- IdenticalText: the entire decoded text is equal, but the raw files differ. The
  status identifies each side's actual UTF-8/BOM or UTF-16 byte order/BOM encoding.
- Different: a validated raw-byte witness for image inputs and pairs compared as
  bytes (a differing byte or a length difference), or unequal complete decoded text
  for text inputs. Equal displayed pixels never imply file identity.

Initial Pending frame publication and mailbox consumption precede all added
whole-input identity work. Decoded text proof compares at most 64 KiB of the two
texts already retained for rendering in one turn.
Raw verification reads at most 64 KiB per source per turn (128 KiB of transient
buffers), with immediate-only process-wide I/O admission. It rechecks the current
attempt between reads and validates both captured revisions before a verdict.
After a progressing turn, the worker polls input without waiting before another
bounded turn. Idle, unconsumed-frame and unavailable-I/O states use a capped
nonzero wait; failed immediate I/O admission cannot create a polling spin.
There is one verifier per admitted comparison, at most 32 pending inputs, one
latest output and one superseded frame awaiting backend reclamation.
Verification runs on the existing backend comparison worker;
there is no identity thread or unbounded work/result queue. Source replacement,
retry, close and validation failure discard pending work and its verdict. Native
paths remain raw `PathBuf` values and are never reconstructed from display names.

A pair compared only as bytes (not admitted as text, or not two images) gets its
verdict from the byte comparison's own scan instead of a second raw pass: at most
256 KiB per source per turn (the native filesystem's largest read, 512 KiB of
transient buffers), with the same immediate-only I/O admission, current-attempt
checks between reads and validation of both captured revisions before the verdict.
The Hex view of a text pair scans the same way but never changes the text's verdict.

This is a LiveValidated read, not an immutable filesystem snapshot. Metadata
checks have the same concurrent-writer limits as the captured-content service.
A text source whose revision moved only in its change time (NTFS ChangeTime or
Linux ctime; same volume or device, file ID or inode, length and write time) is
replaced by a fresh capture of its path when that capture's complete content,
read under the initial read's limits, equals the retained text; whole-input work
then restarts on the new pair. Image sources and every other revision change
still fail as changed.

Implementation and test execution are separate gates. The API is published here
for UI integration; no build or test pass is claimed by this contract document.
