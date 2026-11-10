# Copy/Move decisions and completion

The initial implementation lacked interactive I/O error recovery. Add an explicit
worker-owned failure pause to the existing bounded transfer queue, separate from
overwrite consent. It carries job identity, a monotonic decision version, phase,
raw error/source/destination paths, and actual retry/skip capabilities. Waiting
does not occupy a blocking worker or an I/O permit; cancellation remains active.

Retry resumes only a known safe checkpoint. A failed partial write retains its
source block, rewrites from the committed destination offset and updates the
source digest only after a complete write. Read retries need an explicit provider
capability to reposition the held, revalidated source. Sync/readback retries retain
full SHA-256 verification. Identity changes, unsupported capabilities, integrity
failures and uncertain publication must never be treated as transient I/O.
Preparation retains already opened native objects until retry/cancel; it must not
silently substitute another object at an earlier source/destination path.

Offer Retry, Skip, Skip all of this error type and Cancel only where the worker
supports them. Skip All is scoped by phase, error kind and operation within one
job; it does not grant overwrite consent. Enumeration is non-retryable unless a
provider explicitly preserves the failed entry/cursor: skipping it retains the
source subtree/ancestors for Move. A completed target or uncertain native mutation
must never trigger automatic re-copy or source deletion. Move finalization errors
keep already verified destinations and report retained source/residual locations.

An error is displayed with an icon and separate paths. Decide later dismisses the
window while retaining the queue's attention state; Resolve reopens the exact
versioned decision. Queue refusal retains the decision; stale or disallowed replies
cannot advance work. Successful Retry does not turn a completed job into failure.
Skipped errors remain visible in the terminal record.

On successful Copy/Move completion, close only progress opened automatically for
that job, and only when no other active work or decision needs the window. A
manually opened Jobs window stays open. Error/cancel outcomes remain visible.
Completion never removes the history entry and does not reopen a success notice.
The user confirmed this manual/automatic distinction on 2026-11-08.

The shared decision dialog keeps the currently displayed job/version when another
job needs attention. Error actions are separate from replacement/merge actions;
preparation errors can expose only Retry and Cancel when skipping is unsafe.
Outside clicks do not dismiss errors. Explicit Decide later or Escape leaves the
same decision available through Jobs, including its full diagnostic and paths.
Retry/Skip actions stay outside the diagnostic scroller, including at 320×200;
their action row can scroll horizontally without moving the diagnostic text.
An accepted decision restores the Jobs panel that was open before the prompt;
manual ownership survives this temporary suspension. Closing Jobs and reopening it
manually clears automatic ownership. Other jobs admitted while an automatic panel
is open must also finish successfully before that panel can close automatically.
Its owner outcome and any terminal error/cancellation stay in a constant-size
session record independently of the sixteen retained history rows. Accepted
cancellation immediately marks the local job as cancelling; delayed decisions and
nonterminal progress cannot revive it before the terminal outcome arrives.

Reference behavior and deliberate safety differences are documented in the
[Salamander audit](../../verification/2026-11-08-transfer-error-reference-audit.md).
Validation uses injected I/O failures, full output/checksum assertions, stale and
cancelled decisions, source/target changes, no-replace publication and UI events.
Results and runtime qualification: [verification](../../verification/2026-11-08-transfer-errors.md).
