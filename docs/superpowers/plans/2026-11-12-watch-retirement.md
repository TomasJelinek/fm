# Watch task and retained scope retirement

Approved scope: LIFE-01 watch ownership at b9f8ee4. Preserve native subscription
thread ownership, scope replacement, cancellation, no-follow probes and fixed bounds.
Root owns EngineLifetime/lib startup integration; this task owns watch files/tests.

1. Reproduce final mounted-scope destruction on the caller before first task poll
   and after native-loop unwind, using real Session/NativeProvider captures and
   gated destructors. Also reproduce the unpolled service-owner boundary.
2. Reserve two startup payload slots, one per Shared owner; no new waiter or
   service reservation. Pass `watch::Retirement` and the existing TaskLifetime
   into Controller before filesystem/native ownership is accepted.
3. Store `Arc<WorkerOwned<Shared>>` throughout controller, handle and callbacks.
   Anchor the outer task and nested native closure independently. Keep native
   subscriptions local to the native loop and retain its normal explicit drain.
4. Prove gated scope destruction retains its slot and subscriptions retain their
   creation thread. Reuse the TaskLifetime regressions for uninvoked closures and
   native work surviving runtime shutdown. Run focused Linux RED/GREEN and the
   watch boundary suite, then the parent-approved final affected gate. Record
   exact evidence and request independent review. No commit or push here.

Qualification complete: three observed Linux RED failures became GREEN; the watch
suite passed 39/39 and the final affected Linux filters/integrations plus strict
engine all-target Clippy passed. Independent review found no actionable issue.
See [the verification note](../../verification/2026-11-12-watch-retirement.md)
for commands, counts, timings and artifact identities. Root owns integration.
