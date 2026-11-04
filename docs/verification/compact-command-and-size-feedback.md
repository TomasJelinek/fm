# Compact command row and size feedback

This follow-up starts at `2de9d14` and includes the user's final presentation
and literal-path insertion requests. It supersedes the wrapped command path and
file-row hover behavior in the earlier
[native-controls record](native-controls-and-full-path.md).

The command picker, complete cwd, editor, Options and Run share a single row.
Ordinary paths use their natural width; extreme paths pan horizontally without
elision or changing the underlying native path. File/folder hover stays quiet;
drive, header and control tooltips remain. Footer icons and labels share their
vertical center in both density profiles and at large fonts.

Running folder totals use amber/bold text. Completed partial totals retain `≈`
with ordinary styling. Read-only probes under the actual Windows desktop token
found `C:\Windows\System32\drivers` completes in 35 ms alone and reaches its final
event in 246 ms in the full System32 batch. It reports 175,438,846 logical bytes,
769 files and one denied-access error in `DriverData`. The full batch completes
in 1.791 s on this host. These are observations, not performance guarantees or
an immutable snapshot. No scheduler change was justified; the observed mismatch
was the completed partial result's active-looking presentation.

Windows also supplied an opaque, uniformly black semantic Settings bitmap.
The Windows adapter rejects exactly that semantic placeholder, retaining the
application vector. Native file/drive art, transparent black glyphs and genuinely
colored icons remain accepted.

Literal path insertion uses bare simple tokens or escaped double quotes,
without surrounding apostrophes. Actual Windows PowerShell argument tests cover
33 Unicode, numeric-name, smartquote, whitespace and inert-injection cases. CMD
tests cover 18 native argv cases with a following sentinel and refused unsafe
inputs, including the builtin/CRT ambiguity for quoted trailing backslashes.
An OS-separated `/bin/sh` test covers 30 exact-byte/argument-count cases.

Behavioral RED evidence preceded each change: footer pixels were above center,
file-row hover painted an unwanted popup, the command bar occupied two rows,
completed partial values used active styling, the semantic black placeholder
was accepted, and simple shell paths had surrounding quotes/apostrophes.
The matching regressions exercise rendered pixels, real pointer/scroll input,
model completion state and actual shell/argv parsers rather than source text.
The original late drag regression also exposed implicit centering of the cwd
viewport inside its middle slot: it overlapped the editor, which received the
press. Explicit left/top anchoring and a non-overlap invariant across the full
font/density matrix address that geometry cause; the actual drag test is retained.
The resolved geometry, literal insertion and Windows placeholder changes received
source review with no remaining actionable P1/P2 findings. The validator author
and the root reviewer had separate review scopes; actual parser and input tests
provide the behavioral evidence.

Windows x64: **406 tests passed**, zero failures across 50 suite summaries.
Three subprocess fixtures are directly ignored and exercised by passing parent
tests. Workspace formatting, strict all-targets Clippy, the application build,
the 156-file/100000-byte source guard and backend smoke all passed.

Native Windows FemtoVG-WGPU smoke passed. The inspected
[current screenshot](../screenshots/compact-command-windows.png) shows the full
working path in the single command row, a separate usable editor, centered footer
contents and the corrected Options vector alongside native colored host icons.
The normal FM 0.1 window was reopened in the actual desktop session and verified
visible/responding. Linux UI verification uses the headless software renderer.

Linux workspace: **379 tests passed**, zero failures and zero ignored tests
across 50 suite summaries. Strict workspace all-targets Clippy, application build,
the 156-file/100000-byte source guard and backend smoke also passed. The final
source hashes match the frozen sources used for both platform checks.

Local diagnostics and build products remain in ignored `.tools/`,
`.superpowers/` and `target/` directories. Final logs use
`.tools/final-controls-{windows,linux}-*`; source hashes and native-window evidence
remain in `.superpowers/sdd/final-controls/`.
