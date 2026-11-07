# Selected directory size scans

Ctrl+Shift+S captures the current immutable selection at its exact acknowledged
revision. With a selection it measures only selected ordinary directories; a
selection of files does not fall back to measuring every directory. With no
selection it retains the existing whole-panel scan. Pending selection gestures
use the same bounded capture mechanism as other explicit selection actions.

The command carries the captured snapshot to the backend. Native directory roots
are intersected there with raw selected paths, excluding directory links and
symbolic links. Later selection changes and sorting cannot change the captured
scope or cancel a running scan. Explicit repeat/cancel and navigation preserve
their existing cancellation behavior.

A selected scan retains cached totals for unselected directories in both the
presenter and backend sorting cache. Queued/running styling applies only to the
captured directories. Result mapping continues to use the original scan identity
and native paths across sorting and refreshes.

Actual filesystem errors (`FolderSize.errors > 0`) have a distinct error state
and a visible error glyph in detailed, brief and thumbnail views. Merely queued,
unfinished or cancelled estimates retain their existing calm/active styling and
do not gain an exclamation mark. No file-row tooltip is introduced.

Tests cover pending revisions, selected directories, files-only and empty
selection, Unicode paths, excluded links, preserved caches, changed selection and
sorting, and the distinction between actual errors and unfinished estimates.

## Verification state before the requested build-directory reset

Windows regression tests reproduced four missing behaviors: a pending selection
started scanning all directories; a selected scan discarded an unselected cached
345-byte result; a files-only selection marked directories as running; and an
actual filesystem error shared the unfinished state instead of a distinct state.
The actual Slint `size_errors` test also failed because zero error glyphs were
rendered where one was required. Existing related tests passed in that red run
(18 passed, 4 failed in the library `size` filter).

The captured-selection protocol, presenter dispatch, backend root filtering,
unselected cache retention and distinct error state were implemented after those
red results. They have not yet been compiled or verified. Error glyph UI is also
implemented; the newly requested selected-folder status aggregate remains in
progress. All
build/test sessions were stopped/completed before the user's explicit request to
delete the entire `_build` directory; rebuilding is coordinated by the root task.

## Selected-folder status totals

The status bar keeps selected regular-file bytes separate from directory totals.
For selected directories, it shows the sum and count of completed, error-free
measurements, plus explicit counts for partial and unknown measurements. Partial
bytes, if shown, are labelled estimates and never added to an exact total. A
directory link remains unknown because scans intentionally do not follow it.
An unknown file length and an aggregate exceeding `u64` remain explicitly unknown.

Backend aggregation is seeded once for each immutable selection revision using
the existing native-path folder cache. Subsequent size results replace the old
contribution for that one path. Reading the resulting small summary performs no
listing, selection, or cache walk on the UI thread. Native paths and directory
kinds control membership; display names and recycled entry IDs do not.

Summary events carry their navigation, selection revision and monotonically
increasing summary revision. The presenter accepts only the current selection
and ignores an older summary arriving after a newer one. Pending selection
gestures show an updating label instead of reusing a preceding selection's sum.
Scan cache reset and navigation invalidate the corresponding cached summary.
Unselected cached sizes survive a selected scan and remain available when the
user later selects those directories.

Verification must cover mixed files/directories/links, completed and partial
values, unknown entries, replacing a contribution without double counting,
overflow and later recovery, Unicode/native identity, selection changes during
a scan, and out-of-order summary delivery. Integration followed the initial clean
release build, keeping that build's source inputs stable. Requests distinguish
the current UI navigation from the displayed source generation, so failed
navigation or sorting still uses the preserved cache. Request identities survive
tab activation.

### Backend synchronization

A small request separate from selection mutation captures the acknowledged
selection. This also covers selections produced by comparison, which do not
arrive through the normal selection worker. Each pane keeps at most one active
summary computation and one latest pending request; replacing a pending request
does not spawn another worker. The presenter retries command backpressure and
shows the summary as updating until the exact request is acknowledged.

The observer pins the existing shared folder cache by `Arc` identity. Building
the aggregate takes the cache lock before the observer lock. Result observers
take those locks in the same order and read the current cached contribution,
rather than trusting a possibly delayed notification's value. This prevents a
queued update from regressing a newly seeded total. Events contain a monotonic
summary stamp because delivery after unlocking can occur out of order.
Cache contributions update immediately, while publication keeps just the latest
summary for each pane and sends at most once per 250 milliseconds. Root completion
does not bypass that limit. The final total remains queued until the next slot;
backpressure cannot accumulate one status event per root or retain an unbounded
history. Initial selection results use the same limiter.
Seeding checks the snapshot's cache identity again and holds that identity through
publication, so replacement during the first computation cannot strand the
observer on an obsolete cache.

A replacement scan allocates a new cache, preserving unselected values. Rebuild
the observer against that replacement cache and reject later writes from an old
cache identity. Same-location sorting can keep the shared cache, while a new
navigation or new selection request supersedes the prior observer. No observer
lock is held while sending through the bounded UI event channel.

The reducer keeps completed and partial contributions in separate exact integer
accumulators. Conversion to the public `u64` total is checked; replacing a large
contribution can recover from an earlier overflow without rescanning the entire
selection. The UI receives only counts and optional bounded totals, never the
reducer's per-path map.

Archive selections use the provider's cached selection counts and checked file
bytes. Archive directory totals are explicitly unknown. Native cache requests
and native directory scans are suppressed for virtual panes.

## Verification after the clean rebuild

The backend reducer first failed all five new behavior tests, and UI status tests
reproduced the missing separate fields. A further core regression showed mixed
selections incorrectly made regular-file bytes unknown; folder bits are now
excluded from that separate sum. New-tab request identity reuse and restored
cache lookup after navigation failure were also reproduced before correction.

Windows core library tests passed 24/24. The final engine library passed 174 tests
with one existing opt-in test ignored, including cache replacement during the
first summary computation. Five end-to-end engine tests passed for selected root
filtering, preserved sorting cache, totals following global and individual scans,
rejection of late native scan requests in unavailable archive panes, and bounded
status publication. The UI library passed 360/360 after archive, status and media
integration. Actual Slint metadata
editor and folder-error glyph integration tests passed after the clean rebuild.
Rendered detailed and thumbnail views were inspected: only actual errors show
the glyph. Linux core tests passed 24/24; Linux native platform tests passed
100/100, with 14 existing opt-in Xvfb/locale tests ignored. Linux engine checks
then passed all five selected-scope integrations, all seven summary/cache tests,
and all five aggregate tests. The added publication regression first reproduced
a burst of root-completion totals, then passed after latest-value coalescing.
Paused-clock checks cover the exact 250 ms interval, independent panes, stale
versions and bounded publication under channel backpressure. The broader size
suite exposed shared-gate fixture interference during media integration; its
owner added isolated test gates, and the full focused Linux suite then passed
39/39. Strict Linux engine library Clippy passed with warnings denied. The final
Linux UI library passed 357/357 and all 28 requested rendered-interaction tests
passed. The Linux detailed-view render confirms that the error glyph is aligned
with the failed row and absent from running and cancelled rows. A strengthened
streaming-index regression also passed on both platforms: a directory link stays
in the separate folder count, while a file symlink keeps file bytes unknown even
when its native metadata supplies a length.
