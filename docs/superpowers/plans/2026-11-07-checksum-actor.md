# Checksum engine actor integration

Approved by the goal coordinator after the standalone backend increment. Root
owns UI and release integration; this task changes backend/protocol only.

Commands: `StartChecksums`, `StartSelectedChecksums`, `CancelChecksums`,
`ReleaseChecksums`, `SaveChecksumManifest`, `CancelChecksumManifest`.
Events: `Checksum`, `ChecksumRejected`, `ChecksumManifest`. Save events identify
the exact retained job/generation and a monotonic, nonreused save ID.

The engine adapter reserves the process-wide job/memory lease before queued
admission. Selection expansion runs on a blocking worker and refuses a missing
captured selection rather than substituting the focus. It supplies optional
captured sizes; native identities remain backend-established. Current-generation
results remain in backend state for export. Released jobs cannot be resurrected;
new job IDs increase during an engine session, and generations increase per job.

Calculation is read-only and uses the shared I/O budget. At most two manifests
are pending/active, including mutation-gate waiters. Save cancellation has its
own token and leaves calculation results intact. A blocking save owns the shared
mutation permit through native calls and stage cleanup. Normal engine shutdown
cancels and awaits checksum cleanup before aborting unrelated async tasks.

- [x] Write failing engine-command integration tests; observe missing-contract RED.
- [x] Add typed selected capture/save lifecycle contracts and narrow engine hooks.
- [x] Implement bounded admission, generation retention and independent save cancel.
- [x] Add deterministic queue/backpressure/native-call/selection ownership tests.
- [x] Run Windows/Linux unit/integration and strict Clippy with coordinated slots.
- [x] Review final evidence and hand off UI contracts and remaining limitations.

Do not mark Ctrl+F8 or the modeless results workflow complete in this increment.
