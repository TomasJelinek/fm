# Clipboard worker retirement

Date: 12 November 2026. Follow-up to baseline `b9f8ee4` and LIFE-01.

## Reproduced failures

A fresh Windows engine test binary ran three lifecycle probes against unchanged
clipboard production code. Two failed: dropping an idle Controller destroyed its
final service on the caller; dropping a started Controller with a full, live result
queue left the native pump running. The queued-worker cancellation case already
passed and is retained as additional coverage, not claimed as a reproduced defect.
The RED run finished in 2.03 s. Its pump case explicitly cancelled the old worker
after observation, so a failed assertion could not hang runtime teardown.
Log: `_build/qa/desktop-lifetime-20261112/clipboard-red.log`.

## Implementation

Startup reserves two payload slots and one waiter: one private sixteen-entry inbox
and one serial rejection. NativeWorker declares its receiver before native service
and TaskLifetime anchor. The guard exists before accepting requests and moves into
the existing blocking closure. Thus a cancelled, never-started closure still retires
queued captured/generated sources off the executor. A local ClipboardSession stays
outside that Send guard: creation, pumping, publication and destruction occur on
the same native thread, including unwind. Request ownership ends before result
backpressure; this does not delay provider release until the UI receives its event.

Controller Drop cancels the pump, including its existing send_event retry loop.
The shared TaskLifetime also protects idle service destruction and native work that
outlives its parent. Rejected requests use retirement acknowledgement instead of a
new cancellable Tokio cleanup closure. Plain failure data is built without moving
its source; it is sent only after actual cleanup. Retirement panic remains an error.
Cancelling acknowledgement does not cancel cleanup or free its occupied slot.

The private serial submit future may wait for a free rejection lease while holding
raw input. EngineRun protects that dispatcher until adoption; this is not a public
API that permits an arbitrary unguarded caller to cancel that handoff. Its private
sender exposes only try_send and never exports reserved-send permits.

## Regression and review scope

The additional gated rejection case covers both CapturedClipboardFiles and
GeneratedTextSource, checks cleanup on another thread, checks the occupied domain
slot before and after cancelling the acknowledgement, and checks that no late
refusal or Tokio task is created. The pump fixture records and compares its native
open, write and Drop thread IDs. Existing native-copy intent, Unicode, source-change,
request/global cancellation and output-backpressure tests remain part of the
focused affected gate.

Independent source review found no additional actionable ownership gap in the
changed clipboard boundary. The first focused Windows GREEN gate passed 36 cases
in 0.47s. Final stable qualification passed 935 Windows engine unit cases (seven
existing ignored) and 30 affected integration cases, including all three clipboard
integrations. Linux passed 35 clipboard-filtered unit cases and the same three
clipboard integrations, within its broader affected gate. Strict engine all-target
Clippy passed on both platforms. Full batch results, the test-only desktop fixture
follow-up and deployment evidence are recorded in the
[native-service qualification](2026-11-12-native-service-retirement.md).
