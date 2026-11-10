# First-level tree expansion and Network ordering

The reported filesystem-budget error was reproduced through the production Engine
using a native temporary directory with one parent and 40 child directories. The
initial expansion succeeded, but the resulting visible-row hint queued eight
background subfolder probes and rejected the ninth with the exact reported error.
Two session regressions also demonstrated that a full background queue could reject
an explicit expansion and that an ordinary large viewport failed admission.

Background probes now defer on queue saturation. An explicit expansion/refresh may
replace one queued probe and is executed first. The eight-request limit and shared
worker permits remain intact; eight explicit requests still reject a ninth.
Publication-driven viewport updates progressively schedule the deferred folders.
No filesystem, path, metadata or cancellation guard was relaxed.

Network Neighborhood now follows all drives, ahead of local special folders, in
both panel drive bars and the drive dropdown. The position is consistent while
desktop discovery is incomplete. Native locations retain locale sorting and
deduplication; Hot Paths retain their numbered slots.

Artifacts are under `_build/qa/tree-budget/`. `red.log` records the session and
ordering regressions; `native-red.log` records the actual native-folder Engine
failure.

## Verification

- `cargo test -p midnight-engine`: **767 passed, 1 existing ignored test** across
  library/integration/doc-test groups. All 14 production tree-controller tests
  passed, including first expansion of the native 40-subfolder fixture.
- UI library: **653 passed**; Network navigation integration: **4 passed**,
  including both panel toolbars, dropdown ordering, partial discovery and activation.
- Workspace formatting and the scoped whitespace check passed. All 1,533 checked
  handwritten source files remain below 100,000 bytes.
- All-targets Clippy completed successfully for engine/UI without new findings in
  this change. It still reports 13 existing unused inline-tree integration symbols
  and one existing cloned-reference assertion in `file_list_output_tests.rs`; this
  is not a claim of a warning-free workspace or a strict `-D warnings` pass.
- Independent queue review found no correctness or safety issue. Running callbacks
  retain their permits; deferred checks do not weaken stale-result validation.
- The debug application rebuilt successfully. Its 88,318,464-byte executable has
  SHA-256 `830fdb3d99e9f821002130e3fd24addc26f6b624ce61c4b1a0e4e536ba16d57e`.

Salamander's `src/drivelst.cpp` Network Neighborhood construction was inspected.
The requested position after drives is explicit here; no source or assets were copied.

## Native deployment

The exact debug build was registered with a fresh private installation containing
579 verified plugin files. Startup smoke tests passed from both the private
installation and `_build/debug/me.exe` (exit 0, empty stderr, snapshots produced).
Existing installation contents and user preferences were verified unchanged.
The debug snapshot was visually inspected: both panels place Network immediately
after C:/D: and before the local special folders. Evidence: `runtime-deployment.json`,
`native-candidate.json`, `native-debug-build.json` and their PNG snapshots.
