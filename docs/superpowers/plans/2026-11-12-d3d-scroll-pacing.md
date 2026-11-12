# D3D viewer pacing implementation plan

> **For agentic workers:** Use the executing-plans skill to implement this plan task by task.

**Goal:** Resolve SCROLL-02 using measured attribution of sparse D3D viewer frames, preserving GPU
and software rendering, exact pixels, bounded source reads and the existing performance gates.

**Architecture:** First distinguish probe callbacks, OS window state, CPU render work and GPU
submission/presentation. Reuse bounded owned-process diagnostics. Make a production change only
after a reproducible hotspot and a direct regression or causal A/B identify the affected seam.

**Tech Stack:** Rust, Slint 1.18.1, pinned FemtoVG/WGPU, Windows native diagnostics.

**Spec:** SCROLL-02 in docs/IMPLEMENTATION_BACKLOG.md and the corrected native results in
docs/verification/2026-11-12-scroll-thumb-continuity.md.

## Constraints and ownership

- Starting at 5388a97 in D:/experiments/dev/FM. No commits by this agent.
- Own viewer renderer/UI/probe; coordinate vendored renderer changes before editing.
- Preserve source validation, independent plugins, visible-only paging, reversal and absolute seeks.
- Do not weaken sample guards, force software as the solution, or assert monitor scanout from CPU
  submission observations. No new dependency or speculative resource pool.
- Coordinate shared Cargo and quiet native slots. Output under _build; no package/receipt changes.

## Steps

- [ ] Reproduce exact BAT/Markdown geometry with external owned-window foreground observations;
  compare bounded trace on/off, and distinguish probe from actual application behavior.
- [x] Attribute slow intervals using existing local symbols or narrow opt-in phase diagnostics.
  Separate allocation/upload, CPU composition, queue backpressure and presentation.
- [ ] Record the smallest justified production seam and RED/causal evidence before implementing.
- [ ] Implement the bounded change and run direct correctness/pixel/ownership regressions.
- [ ] Run matched quiet before/after samples, retain failures/outliers and request independent review.
- [ ] Qualify focused lint/tests and document remaining limits with exact binaries and commands.

## Review focus

- Completed GPU resources must not be reused while in flight; queued commands retain ownership.
- CPU and GPU pixels must remain equal, including clipping, alpha and fractional translation.
- Diagnostics are bounded and off by default; measured callbacks cannot hide their own overhead.
- Foreground, visible/occluded state and adapter evidence must accompany causal claims.
- Refill, idle restart, reversal and scrollbar dragging remain distinct measured workloads.

## Current disposition

Existing-binary trace-on/off reproduction and matching-symbol stack attribution are recorded in
docs/verification/2026-11-12-d3d-scroll-pacing.md. All 55 interior samples from nine slow moving
redraws are DX12 swapchain acquisition waits, before item rendering. Windows LockApp owns the
foreground; guarded clicks never reached another application. Foreground and actual-application
qualification must wait for an unlocked desktop. No production/probe edit or Cargo build was
made. The remaining implementation steps are intentionally pending causal interactive evidence;
SCROLL-02 is not complete.
