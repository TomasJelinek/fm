# Custom Commands separators — 12 November 2026

Add `UserCommandInvocation::Separator` as an inert ordered item with an existing
stable ID and validated name. Retain existing item count, byte, ID, icon and label
validation; skip process executable/working-directory requirements for separators.
Keep `UserCommandProcessInvocation` unchanged so native plans cannot represent one.

Persist a strict `{ mode: "separator" }` invocation alongside existing Program and
Terminal variants. Existing command encodings remain readable. Converting a command
to a separator removes its custom shortcut atomically. On load, prune only stale
bindings whose exact action points to a known separator before final preference
validation; preserve unrelated overrides and reject malformed configuration.

Reject separator preview/execution at actor admission and at preparation before
native capture, pinning or process launch. Tests cover ordered round trips, strict
fields, retained validation/budgets, shortcut cleanup and both actor/direct-plan
refusal. UI kind 2, presentation, add/select/reorder flows and dynamic shortcut
registration are owned by the root/Find agents. This work adds no Program-in-terminal
variant or platform launch behavior.
