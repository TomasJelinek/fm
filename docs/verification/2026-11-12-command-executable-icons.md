# Custom Command executable artwork (CMD-01)

## Scope and reproduced gap

Baseline `280c49a` scheduled only explicit `UserCommand.icon` images. A visible Program command with
no explicit image stayed on the generic Run icon. The targeted selector regression failed with one
requested image instead of two; its terminal/script row remained intentionally generic.

Evidence: `_build/qa/command-icons-20261112/selection-red.log` (targeted build 2m05s, one expected
assertion failure). No command is executed by this feature.

## Implementation boundary

Visible automatic artwork occupies the same 256-total target budget and existing bounded image cache
as panel previews and explicit command images. Explicit images retain priority even on decode failure.
The existing 100ms composition bridge submits only the current automatic view; these rows never enter
the thumbnail decoder. Configuration revision plus stable command ID names each row, while a separate
accepted-request generation rejects stale, hidden and cache-cleared results. Queue refusal retries on
the next pulse; image-cache pressure retries only after the visible view changes.

The existing serial desktop icon actor handles automatic requests between ordinary icon requests.
Cached ordinary duplicates count toward that alternation. Backend admission independently limits the
view to 128 entries, bounded retained string/path allocations, unique printable identities, absolute
NUL-free paths and the existing 1 MiB string/path allocation limit. The outer Vec allocation is separately bounded
to 128 elements. A new
accepted view cancels the previous token synchronously. Native capture, prevalidation, OS artwork
lookup and postvalidation all run in one TaskLifetime-guarded blocking closure; no native capture or
handle escapes in the result. Capture denial, a changed revision, cancellation or unavailable artwork
leaves the generic Run icon. Platform calls may be uninterruptible until they return.

Windows extracts the default executable resource with the documented
[SHDefExtractIconW](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shdefextracticonw),
then uses the existing owned-HICON/bitmap conversion to bounded 64x64 RGBA. Paths remain native Unicode.
Linux matches bounded system `.desktop` metadata to the exact canonical configured executable and
uses the existing trusted PNG theme loader. Exec/TryExec are parsed as data according to the
[desktop-entry specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html);
no shell, field expansion or command execution occurs. Ambiguous, malformed, unmatched, out-of-budget
or SVG-only artwork may retain the generic icon. No new dependency, thread or plugin ABI was added.

Salamander's `CUserMenuItem::GetIconHandle` and background icon reader in `src/salamdr3.cpp` were a
behavior reference for explicit/native/generic priority and revision fencing. No implementation was
copied, and its fixed ANSI/MAX_PATH conventions were not adopted.

## Review and qualification

An independent actor review identified ordinary-queue starvation and retained-capacity accounting;
both were corrected before qualification. The follow-up review found no further issue in fairness,
capture/revalidation, cancellation or ownership. Linux lookup also received a separate read-only review.

Targeted Windows checks:

- Selector RED: `visible_program_without_explicit_artwork_requests_its_executable_icon` failed with
  one requested image versus two. The production selector then changed.
- `midnight-ui` library `user_command_presentation::`: **7 passed**, 0.02s. Includes both new selection
  and generation tests, same-identity size/animation cache retry, and retained explicit-image behavior.
- `midnight-engine` library `desktop::command_icons::`: **4 passed**, 0.08s. Covers capture denial,
  revision change, cancellation, fair progress through a full queue of cached ordinary requests,
  capacity/duplicate validation, stale admission, replacement and controller Drop cancellation.
- `midnight-platform-windows` library `executable_icon`: **2 passed**, 0.78s. Includes extraction from
  a real system executable copied to a Unicode filename, and invalid/cancelled input checks.

Logs: `_build/qa/command-icons-20261112/{selection-red,ui-green,engine-command-icons,
windows-executable-icons}.log`. UI and engine test artifacts were linked by the targeted union build;
the overall build encountered one local LNK1104 when an already-built platform test was run during its
later relink. The process finished successfully; final combined qualification repeats that link without
concurrent executable runs. This was a local scheduling collision, not a source or test failure.

The test-only Desktop constructor was also updated to the grouped service argument after the first
compile identified it. Strict Linux Clippy requested the equivalent derived Default for Pending;
that mechanical change is included in final qualification. The coordinated app bridge gate also passed: `midnight-explorer --bin me thumbnails::`, **11 passed**
in 0.55s (`app-bridge.log`). This includes the actual 100ms timer, rejected-send retry, higher generation
after cache clear, hidden-view cancellation, direct decoder exclusion, and retained existing thumbnail
capture/delivery tests.

Linux platform `icons::` passed **19/19 in 0.10s** on final source, including exact-match, ambiguity,
raw-path, parser/read-budget and existing theme/decoder cases. Evidence:
`_build/qa/command-icons-linux-20261112/final-green.log`. The platform owner records detailed limits and
RED evidence in [Linux executable artwork lookup](2026-11-12-linux-command-artwork.md). A final review
also distinguished missing catalog roots from failed resolution: only NotFound is skipped; inaccessible
or cyclic roots return unavailable. The existing root regression now covers a self-loop symlink.

Final Linux engine command-icon tests passed **4/4 in 0.08s** (build 1m17s), including the final
admission/cancellation regression. Strict engine and platform all-target Clippy passed in **1m20s**.
Evidence: `_build/qa/report-arena-20261112/{linux-final-command-icons,linux-final-clippy}.log`.

The implementation adds no timing benchmark or claim of universal icon availability. Host resource
lookup is optional: unsupported or unavailable native artwork keeps the established Run fallback.
Final application packaging and broader batch qualification are recorded by the integration owner.
