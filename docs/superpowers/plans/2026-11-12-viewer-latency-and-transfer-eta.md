# Viewer latency and transfer ETA follow-up — 12 November 2026

Continue the existing usability/performance work after qualified commit a6d90f3.
No plugin authority, checksum policy, retained-source proof, GPU requirement or
independent plugin boundary changes are authorized by a performance shortcut.

- HTML: the pinned CEF settings explicitly cap OnPaint at 30 fps. Compare the
  qualified 30 fps provider with a 60 fps candidate using the same native probe,
  fixture, viewport and renderer. Preserve bounded frame coalescing and all guards.
  Keep only measured improvements; record residual latency and any regression.
- Images: locate the measured validation-to-first-read and publication delays.
  Investigate restricted worker startup and shared presentation before proposing
  changes. Preserve source validation and software fallback; no JPEG-specific path.
- Transfer ETA: derive backend remaining work from actual verification ranges and
  retained Copy/Move proof stages. Account for per-file samples, retries, pause and
  source removal; do not infer completion from cumulative retry I/O.
- Follow exact-head GitHub run 43117240558, diagnose any actual failure, and retain
  failed logs rather than replacing them with a later pass.

Ownership: root HTML experiment/integration; image-loading agent CI; viewer-scroll
agent image startup; editor-rules agent ETA. Cargo targets and native measurement
slots are coordinated. Run focused tests while editing, then final affected checks.
