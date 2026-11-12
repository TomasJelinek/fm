# Filename colors and Carbon Green verification

Implementation boundaries:

- `midnight-core::FilenameColorRule` stores an enabled flag, Masks/Regex expression,
  case policy, Files/Folders/All target, and optional opaque RGB foreground/background.
- The platform-common compiler reuses `CompiledFilter`, but rejects undecodable
  native names for coloring. Panel filtering still keeps those names visible.
  First matching enabled rule wins; disabled expressions are validated too.
- Rules are limited to 32; each expression keeps the existing 4,096 UTF-8 byte,
  64-mask and 2 MiB compiled-regex limits. There is no new dependency.
- Settings with rules use strict version 14. Earlier settings migrate to no rules;
  default settings keep their existing encoding version. Late initial loads retain
  local color edits and preserve unrelated saved fields.
- One coalescing validation lane and two matching lanes execute on backend workers.
  Immutable results retain the exact source `Arc<[Entry]>`, monotonic request ID,
  and rules identity. UI lookup is by entry ID after source-identity validation.
  Disabling rules or entering a streaming listing cancels and releases prior work.
- Only filename cells receive an optional normal background. The size/progress/error
  columns retain their semantic colors. Focus and selection keep theme backgrounds,
  edge bars and the combined-state top line. Parent navigation is never matched.
- The same Slint presentation functions and constant-size Rust contrast adjustment
  drive rows and four preview states. Adjustment keeps already-readable RGB values
  and otherwise changes lightness toward white/black while retaining hue where
  possible, with a 4.5:1 minimum on the actual background.
- Carbon Green appends theme index 8 and storage name `carbon-green`. Existing theme
  identities remain unchanged. It uses charcoal surfaces with bright green accents.

Reference inspection: Salamander `src/dialogs4.cpp` `CHighlightMasksItem` and the
Colors page were consulted for ordered mask editing and normal/focused/selected/
combined previews. No source code or artwork was copied.

Test results, diagnosed assertions and artifact paths are recorded in
[the verification record](../../verification/2026-11-07-filename-colors.md).
