# Make File List — 9 November 2026

The host implements Ctrl+M from the current selection/caret in native, archive and
expanded-tree views, using immutable identities and displayed ordering. Templates,
two-pass maximum widths, preview, Clipboard/Viewer/File and confirmed append follow
the inspected Salamander MakeFileList workflow without copying its implementation.

Output uses a bounded backend worker and private stream spool. File output uses
verified publication; replacement/append preserves supported security metadata or
fails explicitly. Confirmation binds the target revision and append mode. Viewer
success waits for actual content readiness; native window construction alone cannot
clear the captured selection. Clipboard cancellation is checked again after opening
the native session and before changing clipboard ownership/content.

## Evidence

Artifacts: `_build/qa/make-file-list/`.

- Core/common formatter/preferences baseline: **312 passed, 1 ignored** before
  viewer-association additions. Native Windows transfer **31** and picker **4**;
  native Linux transfer **32** and picker **7** passed during implementation.
- Engine Make File List **23/24** first passed; the remaining regression exposed an
  inherited Windows DACL losing protection. Descriptor control is now made explicitly
  protected before application. The final native preservation subset **4/4** and
  verified output subset **7/7** pass, including the original failing append/replace.
- Clipboard generation/cancellation **8/8** passes. Generated-source capture and
  actual viewer-readiness coverage run in the full viewer-engine suite.
- Independent review corrected nested archive-tree paths, asynchronous viewer
  readiness, stale generation completion and the startup plugin preference guard.
- Strict Clippy passes for core/common/Windows platform/viewer engine/renderer.
  Including the whole engine still reports 13 unused inline-tree symbols from its
  separate unfinished integration; these warnings have not been suppressed.

Main UI: **653 library tests passed**. Standalone Make File List window **4**,
key/menu/toolbar dispatch **1**, F10 menu **5**, keyboard/settings **20**, and
viewer-association UI **3** integration tests passed. Desktop and small-window
software snapshots were inspected. Formatting passes; **1,533 handwritten source
files** pass the 100,000-byte source-size check.

The final debug application build completed successfully. Strict all-targets
Clippy (`--no-deps -D warnings`) also passes for the modified UI, core,
platform-common, viewer renderer, viewer engine and viewer UI packages. The
workspace-wide whitespace check reports an existing extra blank line at the end
of `filesystem-api/PROTOCOL.md`, outside these changes.

The final 88,322,560-byte debug executable has SHA-256
`dc41332860d2ffa870211ba6cba552e5679209ebfbce19787629044936f0e9d8`.
It was installed with 579 verified plugin files into a new private runtime and
registered for this exact build. Both the private executable and
`_build/debug/me.exe` passed native startup smoke tests (exit 0, empty stderr,
rendered panel snapshots). Existing installations and user preferences were
verified unchanged. Evidence: `runtime-deployment.json`, `native-candidate.json`
and `native-debug-build.json` under the artifact directory. The candidate snapshot
was visually inspected; both panels and the command/footer rows rendered.

This record does not claim clean workspace-wide validation or completion of the
broader product goal.
