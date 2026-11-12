# Desktop native/service lifetime qualification

Baseline: b9f8ee4. Status: implemented and qualified on Windows and Linux.

## Ownership changes

Desktop receives the existing TaskLifetime and anchors both asynchronous service
work and every independently surviving native closure. The guarded service owner
is shared; icon pixels, location/property metadata and editor configuration remain
plain return values. No platform launch, decoding or path policy changes.

Terminal, editor and Custom Commands share the original eight process permits and
the same eight-slot ProcessTracker. Desktop openers retain their independent
sixteen permits and a separate fixed sixteen-slot tracker created at startup.
The implementation has fixed backing arrays, no queue growth and one plain worker
per tracker. Admission happens before launch. The returned native child enters
TrackedProcess inside the native closure, before id() or return. This protects
completed unconsumed outputs, failed/cancelled tracking sends and reaper shutdown.
Normal opener completion/error events still use the existing async reaper.

A ticket follows its native child until observed exit and actual destructor
completion. Errors or panics while waiting do not prove exit or return quota.
Child destruction precedes service-anchor release and process quota release.
There is no implicit kill and no process polling on the shared retirement worker.
A native child or destructor can outlast shutdown; ownership and admission remain
bounded rather than inventing a finite exit-time guarantee.

Editor input uses five retirement slots: four accepted launch permits plus one
serialized intake. One startup waiter acquires the guard before accepted/rejected
handoff; EngineRun protects incoming payloads while that intake waits. The same
Work { target, launch } guard remains intact throughout dispatch, with no raw
into_parts handoff on the dispatcher. Only the native worker extracts it while
retaining its lease through actual destruction/unwind. Target precedes services,
service anchor and permits in payload destruction order. Rejection acknowledges
plain-worker retirement and reports a cleanup panic instead of hiding it behind
the original launch refusal.

## RED evidence

`_build/qa/desktop-lifetime-20261112/windows-red.log` reproduces both original gaps:

- Unpolled Desktop service tasks released the final service on the executor
  (ThreadId(4) equalled the caller).
- A failed normal tracking send destroyed a still-running ordinary terminal.

Result: 0 passed, 2 failed, 929 filtered, 0.05 seconds. Production constructors and
launch paths were unchanged for that build. The same artifact also reproduced the
parent's clipboard RED cases in clipboard-red.log (1 passed, 2 expected failures).

## Qualification

The first implementation passed desktop10/10 (0.67 seconds) and the shared
clipboard filter36/36 (0.47 seconds), recorded in windows-green-first.log and
clipboard-green-first.log in the same QA directory. These precede the final Work
simplification and additional regressions; they are not a final-source claim.

Final regression coverage includes:

- Service owner cancellation before first task poll.
- Tracking receiver loss and abort after launch with a full tracking channel,
  for all three ordinary launch paths.
- A complete editor launch output whose delivery future is never polled before
  runtime destruction; process ownership survives without Tokio.
- Opener success completion while every terminal tracker ticket is occupied.
- Independent eight/sixteen ticket exhaustion and return.
- Queued captured editor work after runtime destruction, and blocked target Drop
  retaining the admitted lease and both launch/process permits.
- Captured-source destruction panic reported as an editor cleanup error.

The viewer agent's independent initial and final read-only reviews found no
actionable production ownership/quota issue in process adoption, nested anchors,
fixed admission, the Work simplification or cleanup-panic reporting. Its evidence
caveat is retained: the full-channel abort test waits for launch notification and
30 milliseconds, rather than instrumenting the exact Send::Pending poll. It
covers that broader abort boundary; the completed-output editor test instead uses
a single-blocking-worker barrier proving native completion before delivery.

The first post-review Linux compile found a diagnostic-lifetime type error:
editor_failure accepted a dynamic string while editor_error still required a
static one. The helper now owns its message in io::Error; this changes no cleanup
or launch behavior. Final-source test/Clippy results follow.

## Combined Windows gate

The parent ran the completed implementation once with all engine unit tests and
eight directly affected integration targets. The inspected log
`_build/qa/native-services-20261112/windows-final-tests.log` reports 935 unit tests
passed, 7 existing ignored, and 30 integration tests passed with no failures.
Strict engine all-target Clippy passed in windows-clippy.log (19.29 seconds).

A subsequent test-only correction removes a possible scheduling race from
editor_capture_cleanup_holds_lease_and_launch_permits_through_actual_drop: its
post-release wait now requires both permit counters and a free cleanup lease.
The native result can legitimately retain its launch permit briefly beyond
payload cleanup. Exact assertions while the native Drop is blocked are unchanged.
The parent reran only that corrected test; it passed (1/1, 0.02 seconds) in
editor-fixture-final.log in the same directory. Production remained frozen.
The final debug application build also passed in debug-build.log.

## Native Linux gate

The watch agent's coordinated fresh gate included the final fixture correction.
The following inspected logs are under `_build/qa/watch-lifetime-20261112/`:

- linux-final-lib-desktop.log: 16 passed, no failures/ignored, 0.46 seconds.
- linux-final-lib-user_command.log: 18 passed, no failures/ignored, 0.48 seconds.
- linux-final-integration-desktop.log: 11 passed, 0.49 seconds.
- linux-final-integration-editor.log: 3 passed, 0.26 seconds.
- linux-final-integration-user_command_actor.log: 2 passed.
- linux-final-integration-user_command_process.log: 1 passed.
- linux-final-clippy.log: strict engine all-target Clippy passed, 1 minute 32 seconds.

The fresh Linux engine unit artifact SHA-256 is
`8be88459fb1b8a22debb76410a75c78d8f468435578caaa9bfc82d875c4db179`.
These were scoped backend qualification runs with fake process/capture lifecycle
gates and existing launch integrations; no native performance claim is made.
No dependency, platform launch implementation, command authority or kill policy changed.
