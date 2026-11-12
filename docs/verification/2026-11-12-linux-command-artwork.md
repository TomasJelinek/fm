# Linux executable artwork lookup

Scoped CMD-01 platform implementation; final targeted tests and strict lint passed.
The engine owner handles captured executable validation, request coalescing and cache
invalidation. This Linux adapter only returns bounded pixels or an unavailable result.

## Matching and limits

The backend reads direct `.desktop` entries in `/usr/share/applications` and
`/usr/local/share/applications`. It does not recursively visit application folders,
scan user XDG application directories, inspect executable contents, or run any
program. Canonical metadata paths must remain in the selected trusted system root.
Existing descriptor-based `read_bounded` refuses non-regular files without waiting
on a FIFO writer.

A lookup visits at most 512 directory entries, accepts at most 64 KiB per desktop
file and reads at most 4 MiB in aggregate, including oversized-file probe bytes, and resolves bare desktop executable names through at
most 64 absolute PATH components (32 KiB total PATH text). Cancellation is checked
before work and throughout enumeration, reads, resolution and theme lookup.
Exhausted enumeration cannot publish a partially searched match.

Only `Type=Application`, a valid primary `Exec`, a safe theme `Icon` name, and an
optional agreeing `TryExec` qualify. Executable matching compares canonical native
paths; equal basenames do not establish identity, and no lossy path conversion is
used. Conflicting icon names for the same executable yield no icon. Absolute
configured executable paths are required. Directory entries, malformed or hidden
applications, field-dependent executable names and unsupported quoting keep the
existing Run fallback.

The parser applies documented desktop string escapes before the Exec quoting rules;
it validates metadata as data and performs no field expansion or shell evaluation.
The conservative grammar is based on the
[Desktop Entry Exec specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html)
and [value escaping rules](https://specifications.freedesktop.org/desktop-entry/latest/value-types.html).

## Artwork resources

Matched safe names use the existing configured/inherited system-theme loader and
its trusted PNG/pixmaps roots, manifest/attempt bounds and bounded PNG decoder.
Absolute Icon paths, user icon directories, scalable-only SVG artwork and unsupported
application formats are intentionally best-effort misses. Existing type/semantic
icons retain their selection order and invalid-size early rejection. No additional
cache, dependency, native worker, process or retained native handle was introduced.

## Evidence

A deterministic Linux RED against the unavailable lookup stub produced two expected
positive-match failures and six passing refusal cases. The first implementation then
passed all 15 icon tests (eight new metadata cases and seven existing icon cases) in
0.11 s; build 51.33 s. Logs and reproducible drivers are under
`_build/qa/command-icons-linux-20261112/{red,green}.log` and corresponding shell files.

Four additional source-frozen edge tests cover two-stage escaping and malformed
Exec syntax, per-file/aggregate read limits, relative PATH/unsafe resource refusal, and failure to enumerate an existing catalog.
These were qualified by the final 19-test run below; the first 15-test pass did
not qualify the later assertions.

The CMD-01 owner's source review found no matching, parsing, trust-boundary or
cancellation issue. A second independent review identified an incomplete-catalog
ambiguity: a readable matching entry could be accepted beside an oversized/unreadable
conflicting entry. The strengthened fixture reproduced that defect (one failed test
in 0.03 s; build 1 m 29 s), preserved in `ambiguity-red.log`. Lookup now refuses an
incomplete read of eligible trusted metadata or a failed enumeration of an existing
resolved catalog. The reviewer accepted both corrections. Aggregate accounting also
reserves the bounded reader's one-byte oversize probe. Final source is frozen for the
combined platform gate described below.

The combined Linux gate first passed 19/19 icon tests in 0.07 s (build 1 m 56 s).
A final review tightened root resolution too: only a missing catalog may be skipped;
other canonicalization errors retain fallback. The existing catalog test now covers
a self-loop symlink without depending on privileged permission behavior.

The final source passed **19/19 in 0.10 s** in
`_build/qa/command-icons-linux-20261112/final-green.log`. Strict engine and Linux
platform all-target Clippy passed with `-D warnings` in **1 m 20 s**, recorded in
`_build/qa/report-arena-20261112/linux-final-clippy.log`. The shared lint gate also
required the equivalent `Permissions::from_mode(0)` to `0o0` syntax correction in
an existing native discovery fixture; production permissions were unchanged.
