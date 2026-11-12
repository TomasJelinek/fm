# Custom command environment aliases — 2026-11-12

The reported Custom Command failure stopped before creating a process with
“Inherited environment contains ambiguous case-insensitive duplicate names.”
The user's existing process environment was not inspected; the screenshot alone
does not establish which names triggered that refusal.

Two actual encoder/capture behaviors reproduce that exact error:

- Windows ordinal case aliases with exactly equal native values, such as `Path`
  and `PATH`, were rejected even though no value authority was ambiguous.
- Capturing an inherited lowercase drive record such as `=c:` added a synthesized
  `=C:` record because the synthesis check used case-sensitive equality. This could
  create a conflict with the captured drive working directory.

The capture now recognizes drive names without ASCII case sensitivity. The block
encoder coalesces ordinal case aliases only when their UTF-16 values are exactly
identical. Different values remain a pre-spawn error; neither first-wins nor
last-wins selection is introduced. Custom override allowlisting, stale contextual
variable removal, native UTF-16 preservation, NUL checks, sorting, double-NUL
termination, and the existing environment budget remain in force.

A real conflict now identifies the two escaped variable names without including
values. Each displayed name is bounded to 128 UTF-16 units; this diagnostic label
is never used as an environment identity or other authority.

## Verification

Focused tests include the production `user_command_environment.rs` and
`windows_argv.rs`, linked directly with cached production core/Windows libraries.
No Cargo build or dependency change was needed during the shared Windows build.
Evidence is under `_build/qa/custom-environment-20261112/`:

- `environment-red.log`: both new alias/drive regressions fail with the exact
  original error; all four existing environment regressions pass.
- `environment-green.log`: all six tests pass after the two corrections.
- `environment-final.log`: all seven tests pass, including the names-only error
  contract, bounded diagnostics, and escaped control characters.

The tests cover three aliases, Unicode ordinal name folding, isolated UTF-16
surrogates in identical values, retained lowercase drive cwd, and rejection of
values that differ only by case as well as unrelated conflicting values. The
current diagnostic child snapshot succeeds; that observation is not claimed as
provenance for the user's already-running application.

The final coordinated Windows Cargo gate passes all seven compiled environment
integration tests (0.00 seconds). The adjacent native custom command integration
gate passes three tests (1.39 seconds), with one owned subprocess fixture ignored
for direct invocation. Those checks include actual native argv/cwd delivery,
pre-spawn cancellation/missing/batch refusal, and native malformed-image error
handling. Evidence: `_build/qa/find-metadata-scroll-20261112/windows-affected.log`.
The complete affected Windows strict Clippy gate and fresh application build also
pass. These results preserve the qualification above: no private environment
values were inspected from the user's existing process.
