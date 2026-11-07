# Metadata Expression Panel Filters Implementation Plan

> For implementers: use superpowers:executing-plans task by task. The user goal and
> root approved this increment. No subagents, commits or live preference changes.

**Goal:** Add Expressions alongside existing panel Masks/Regex without moving
filesystem work or matching into UI callbacks.

**Architecture:** Core carries mode and typed UTF8 diagnostic spans; common
compiles a bounded typed predicate over captured Entry metadata. The existing
engine projection/validation workers and last-valid UI flow consume it. Schema18
persists the new mode while preserving earlier command/checksum/native-path data.

**Stack:** Rust std, existing pinned regex, existing Tokio/Slint/settings codec.
No new dependencies or features. Windows/Linux implementations remain separate;
expression evaluation itself is portable and performs no filesystem queries.

**Requirements:** FILTER-01 in [the backlog](../../IMPLEMENTATION_BACKLOG.md),
main design panel filters/color extension, and the approved detailed plan in
`_build/goal/filters/implementation-plan.md`. Inline-tree ancestor context waits for
TREE-01; flat panel filtering is delivered in this increment.

## Accepted semantics

- Masks/Regex syntax, case policy, budgets, raw-name handling and directory bypass
  remain unchanged. Filename colors and Find stay name-only and do not accept an
  Expression accidentally without metadata.
- Expressions evaluate files and folders, with not/and/or, comparisons, name
  literal/wildcard/regex, kind, logical regular-file size, modified and reported
  native attributes. They never follow links, inspect contents or run a script.
- Missing metadata uses three-valued logic; final unknown does not match. Explicit
  present(field)/missing(field) expose it. Unknown attributes are not false flags;
  Linux dot-names are not inferred as native hidden attributes.
- Date literals require explicit UTC Z or numeric offset, checked Gregorian
  years1..9999 and up to9 fractional digits. Literal and snapshot timestamps compare
  exact UTC nanoseconds without Windows SystemTime literal rounding. No local
  dates, implicit now, arithmetic or locale-dependent interpretation.
- Source4096bytes, tokens512, nodes256, syntactic nesting32, name patterns16.
  Per-pattern compiled64KiB/DFA64KiB bounds are resource budgets, not RSS or an
  allocation-free guarantee. Numeric units and overflow are checked.
- Disabled Expressions still validate syntax/types before being accepted and then
  match every entry; legacy disabled-mode behavior remains compatible.

## Files and steps

1. Core filter.rs + common filter_expression/*, filter.rs: keep existing
   CompiledFilter name-only; add CompiledPanelFilter::matches_entry. Integrate the
   prototype language tests through the actual exported common crate.
2. Engine filters.rs/search.rs: compile once, preserve coalescing/cancellation and
   immutable identity projection, return optional typed diagnostics through the
   existing PanelFilterValidated event. Test stale output and authoritative target
   selection, prefix composition and refreshed metadata.
3. Common settings.rs/settings/user_commands.rs: allocate18 only for Expression
   mode, extend earlier command decoder range and include required empty commands/
   default checksum fields. Test malformed/cross-version data and unchanged legacy
   minimum encoding. Colors explicitly reject metadata modes.
4. UI presenter/filter.rs, commander_presentation.rs and panel-filter-dialog.slint
   with narrow main.slint callback adaptation: explicit mode picker, typed syntax
   help, line/column feedback, preserve draft and last valid active filter, no new
   native I/O. Test actual choice, captured pane, feedback,320x200 and Escape.
5. Run focused backend/codec/projection/UI tests on Windows/Linux with coordinated
   Cargo slots, strict Clippy, formatting/source-size checks. Record actual evidence
   in docs/verification and limits/syntax in docs/developer. Do not treat historical
   direct prototype runs as integrated workflow qualification.

## Review focus

Missing-value negation, invalid native names, timestamp precision/offsets, schema18
required earlier defaults and hidden-row mutation targets each need causal tests.
Invalid or stale validation must never replace the last valid active filter.
Filters use captured listing metadata; refresh/watch/sort creates a new projection
without per-entry filesystem queries from matching or rendering.

Source ownership: root owns shared core/common root exports (common expression
module export was coordinated); filter implementer owns narrow protocol Event and
main.slint filter regions. VFS and Copy metadata work remain in separate files.

## Behavioral reference

Salamander's local `src/filter.h::CFilterCriteria` and
`src/filter.cpp::{PrepareForTest,Test}` were inspected in the reference checkout.
They informed preparing size/date bounds once and distinguishing file sizes
from directory metadata. No reference implementation or assets were copied.
This language uses explicit UTC offsets and missing values rather than adopting
legacy local-date or failed-conversion behavior.
