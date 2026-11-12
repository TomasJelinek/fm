# Desktop lifetime follow-up

Status: implemented and qualified on Windows/Linux; baseline b9f8ee4.

- [x] Reproduce raw service/ordinary process ownership failures with bounded fake-native gates.
- [x] Anchor Desktop service tasks and their nested native closures with TaskLifetime.
- [x] Share the eight terminal/editor/Custom Command process tickets; add a separate fixed
      sixteen-slot opener tracker. Adopt outputs before leaving the native closure.
- [x] Guard captured editor targets before queued work and serialized rejection. Reserve
      five payload slots (four accepted launch permits plus one intake) and one waiter.
- [x] Preserve opener completion events, uncertain-wait retention, no-kill behavior and quotas.
- [x] Qualify targeted desktop/editor/tracker/runtime tests and strict scoped Clippy.

Root owns EngineLifetime/lib startup integration; this agent owns desktop, editor and
process tracker/runtime files and dedicated tests. Existing platform implementations
remain separate and unchanged. No new dependency, implicit killing, per-drop thread or
unbounded queue. Cleanup admission stays occupied until actual native destruction.

Evidence: [desktop ownership qualification](../../verification/2026-11-12-desktop-ownership.md).
