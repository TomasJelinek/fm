# Native Find actions and viewer integration

This implements the authorized guarded Find actions alongside native inline-tree
metadata and the independently measured wheel-refill follow-up. It does not claim
completion of generic-provider Find or the entire application backlog.

## Boundaries

- Find rows carry session, generation and result IDs. Explicit View, Basic View,
  Edit and Open capture the exact retained result; Enter/double-click retain Go to.
- Stop retains result actions; closing or replacing a search revokes new launches.
  Sixteen retained actions share the startup retirement worker, with admission held
  until actual worker-side destruction. No additional cleanup thread is created.
- Internal viewers obtain a bounded reader through the installed native provider.
  Validate the Find identity/revision before and after capture and compare the
  captured provider version. Never infer authority from a displayed path or flat
  listing. Closing the viewer cancels in-flight validation.
- Native Open/Edit validate immediately before invoking the existing platform
  service. External programs consume live paths: this is not an immutable snapshot
  or a guarantee against subsequent replacement after the native handoff.
- Existing native revision formats are compared by platform-specific adapters.
  Unknown formats and malformed identities fail closed. No plugin ABI or new
  third-party dependency is introduced.
- The viewer refresh callback drains pending sessions after the current UI borrow
  ends, sharing the same deferred dispatcher as diagnostic viewers.

## Verification

1. Targeted engine tests for stale/closed/Stopped searches, admission and worker
   retirement; UI tests for current shortcuts, exact row tokens and queue pressure.
2. Platform version mutation/truncation tests and viewer cancellation/source tests.
3. Protected installed-provider test: render the exact Unicode-named Find result;
   replace that path and require a diagnostic without rendering replacement data.
4. Coherent affected-package Windows/Linux tests, formatting and strict Clippy.
5. Fresh debug build and protected deployment; actual application/native foreground
   checks remain distinct from headless and fake-service qualification.
6. Reconcile public behavior and ownership documentation with recorded evidence.
