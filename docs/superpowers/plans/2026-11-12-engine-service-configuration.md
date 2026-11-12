# One-time engine service configuration implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans for this approved, scoped lifecycle follow-up.

**Goal:** Reject repeated service configuration before accepting another native owner, while retaining runtime-independent final cleanup.

**Architecture:** An opaque, exclusive `ServiceSlot` borrows one vacant field in the existing guarded `EngineServices`. Admission accepts only metadata; consuming `install` transfers one owner infallibly into that field. No displaced-owner collection, new worker, retirement reservation or plugin ABI is introduced.

**Tech Stack:** Existing safe Rust, `Arc`, startup `WorkerOwned` service guard, integration tests and current Windows/Linux build gates.

**Spec:** LIFE-01 in `docs/IMPLEMENTATION_BACKLOG.md`, the builder limitation in `docs/verification/2026-11-12-engine-lifetime-review.md`, and `docs/developer/ENGINE_LIFETIME.md`.

## Constraints and review focus

- Acquire `Engine::lifetime()` before constructing provider-bearing services; required filesystem/wakeup construction is unchanged.
- All 17 optional services start unconfigured, including locale. Preserve existing fallback behavior.
- `configure_*(&mut self)` accepts no native owner or factory closure. It returns `Result<ServiceSlot<'_, T>, ServiceAlreadyConfigured>`.
- A successful token borrows the vacant field exclusively. `install(self, T)` cannot fail or replace a prior owner. Abandoning a token leaves it vacant.
- Duplicate admission is metadata-only and preserves the original owner. Do not add an error path that accepts and then drops a replacement `Arc` or closure.
- Network absence means skipping installation; there is no owner-taking clear/None operation.
- Preserve 304 retirement slots and seven waiters; no unbounded history, blocking caller fallback or per-drop worker.
- The internal Rust composition API changes. Plugin APIs/ABIs and native operation authority do not.
- Only Engine setters migrate. Unrelated NativeProvider locale and comparator picker builders remain unchanged.

## File ownership

- Configuration implementer: new `crates/midnight-engine/src/engine_configuration.rs`, new `crates/midnight-engine/tests/engine_configuration.rs`, this plan and scoped verification note.
- Integrator: `crates/midnight-engine/src/lib.rs` removes the old setters and exports `ServiceSlot`/`ServiceAlreadyConfigured`; application and affected test callers migrate to explicit admission/install. Integrator also updates the lifecycle contract/backlog.
- Platform gate owners: serialized Cargo and final qualification. No concurrent Cargo use of the Windows target directory.

## Tasks

- [x] Inspect constructors and every optional setter usage. Locale is `None` at startup; normal application composition installs each optional service once.
- [x] Add and run a real baseline RED integration test: a second Settings installation destroys the first service on the caller without Tokio. Preserve source and log under `_build/qa/engine-configuration-20261112/`.
- [x] Agree the exact one-time admission API with the integrator before shared-file edits.
- [x] Implement the opaque token, metadata error and all 17 typed Engine accessors in the dedicated module.
- [x] Replace the preserved RED test with duplicate-before-construction, unused-token, network omission, gated real-file cleanup and unpolled-run/runtime-shutdown regressions.
- [x] Migrate application/tests and remove all old ownership-taking Engine setters.
- [x] Run the focused GREEN integration under the Cargo lease; repair real failures without weakening assertions.
- [ ] Run affected platform/engine/application checks under coordinated leases; inspect strict lint and formatting.
- [ ] Independent review, update evidence/contracts and report remaining practical limits. No blanket LIFE-01 closure.
