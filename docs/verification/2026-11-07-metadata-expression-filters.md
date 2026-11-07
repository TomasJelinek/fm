# Metadata expression filter verification — 2026-11-07

Scope: Expression mode alongside existing panel Masks/Regex, backend validation
and immutable projection, preference schema 18, and the filter dialog. No new
dependencies, filesystem reads during evaluation, scripts or live preference
changes were introduced. The language and missing-value semantics are described
in [METADATA_FILTERS.md](../developer/METADATA_FILTERS.md).

## Causal tests

Integrated tests first failed against the old panel wrapper/schema/projection:
the metadata wrapper had three failures, schema 18 round trip failed, and two
expression projection tests failed. The actual dialog picker regression failed
because the existing dialog had no mode picker. These failures are retained in
`_build/goal/filters/red-*.log`; the earlier standalone prototype does not count
as integrated workflow evidence. Review also found that opaque archive names
were evaluated through their captions. A new exported-compiler test reproduced
that failure before the conservative raw-name fix; the final suite covers invalid
raw bytes, a mismatched UTF-8 caption, and an exactly matching UTF-8 archive name.

## Windows x86-64

Focused Cargo tests use the repository toolchain, locked offline dependencies,
debug information disabled, incremental compilation disabled and one test thread.

| Boundary | Result | Evidence |
| --- | --- | --- |
| Exported compiler, raw native/archive names, literal/date/type errors, truth tables and resource limits | 12 passed | `final-common-windows.log` |
| Existing Masks/Regex matcher | 3 passed | `final-common-windows.log` |
| Metadata panel wrapper, folder policy, disabled mode and name-only consumer refusal | 4 passed | `final-common-windows.log` |
| Legacy/schema 18 preferences, cross-version refusal, required earlier fields and preserved commands/checksums | 5 passed | `final-common-windows.log` |
| Existing command preference codec | 3 passed | `final-common-windows.log` |
| Typed UTF-8 diagnostic and superseded validation behind a full output queue | 2 passed | `final-validation-windows.log` |
| Projection/prefix/identity/size targets, refreshed metadata, 50,000-row cancellation and native names | 13 passed | `final-projection-windows.log` |
| Fluent Slint source compilation | Passed | `slint-check.log` |
| All UI library units, including draft/stale/queue and hidden-cursor refusal cases | 466 passed | `final-ui-windows.log` |
| Actual filter/comparison dialogs: picker, captured pane, feedback, fonts, minimum size and Escape | 3 passed | `final-ui-windows.log` |
| Actual Copy dialogs and movable/compact dialogs after the shared UI build | 5 + 5 passed | `final-ui-windows.log` |
| Core/common/engine all-target strict Clippy and owned-file rustfmt | Passed | `final-backend-clippy-windows.log`, `format.log` |
| UI library and the three listed interaction targets: strict Clippy | Passed | `final-ui-clippy-windows.log` |

## Linux x86-64

The real exported common crate passed the same 27 compiler/wrapper/legacy/codec
tests, including invalid native bytes and the archive-name correction. Validation
passed 2 tests and projection passed 13, including 50,000-row cancellation and
immutable snapshot refresh. Core/common/engine all-target strict Clippy passed.
Evidence: `final-common-linux.log`, `final-validation-linux.log`,
`final-projection-linux.log`, `final-backend-clippy-linux.log` under
`_build/goal/filters/`. The combined UI batch passed all 463 library tests plus
3 filter/comparison, 5 Copy and 5 movable/compact-dialog interaction tests in
`final-ui-linux.log`. The full core crate passed 92 tests in
`final-core-linux.log`. Focused UI library/interaction-target strict Clippy passed
in `final-ui-clippy-linux.log`. Owned-file formatting, `git diff --check`, and the
repository source-size check also passed (1,011 source files, each at most 100 KB).

## Platform and feature limits

The evaluator consumes the metadata already present in a listing; provider
availability controls which fields are known. Linux native attribute flags are
missing, including for dot-names. Timestamp comparisons preserve reported native
precision. Invalid native names are not converted to replacement characters for
name predicates. Archive captions with unknown/ambiguous text encoding remain
unknown for Expression name predicates; `missing(name)` exposes that state. A
failed or obsolete filter does not authorize operations on hidden rows.

macOS has no native runner in this session and is untested. Inline tree ancestor
context remains dependent on TREE-01. This slice evaluates flat panel snapshots;
it does not create a tree or perform network/content discovery.
