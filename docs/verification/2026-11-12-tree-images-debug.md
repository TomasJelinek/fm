# Integrated tree commands and image plugins — 2026-11-12

The Windows debug application was rebuilt after the producer-side command capture
retirement correction. Its SHA-256 is
`ea19572aa83b8ac2fa20886d538ca63ccbf5700f57fe3ce473816061c124b8d1`.
The protected installation is
`C:/Users/tjelinek/MidnightExplorer-20261112-tree-images-ea19572a`.
An explicit deployment registered that exact build hash for the development EXE;
startup never copies code or relaxes the normal plugin trust policy.

The installation includes the corrected production SVG/AVIF image viewer and
thumbnailer. Their exact hashes and nine-case actual restricted-worker acceptance
are recorded in [image-host qualification](2026-11-12-image-host-formats.md).
Other filesystem/viewer/video packages come from the previously qualified private
runtime. HEIC/HEIF support is not claimed.

After the final retirement fix, both native command integration targets passed:
two controlled-service cases and two real child-process argv/cwd cases. They ran
as two concurrent processes, one libtest thread each, with no ignored tests or
cleanup errors. The individual artifact elapsed times were 24.823 and 28.122 s;
this loaded-machine check is correctness evidence, not a speed comparison.
The successful build receipt and complete results are under
`_build/qa/host-integration-20261112/final-tree-integration-build.jsonl` and
`final-tree-integration/summary.json`.

The first development-EXE backend startup refused with
`Plugin runtime executable changed while hashing`. Both original and deployed
EXE bytes subsequently matched the recorded SHA-256. The first refusal is retained
as `first-start-integrity-refusal.log`. A controlled retry of the identical build
passed backend listing, native UI startup and software UI startup, each with exit
0 and empty stderr. User preferences were unchanged. No trust/revision check was
modified, and the initial transient failure is not claimed fixed without a cause.
A separate exact-handle probe subsequently performed 16 hashes with the same share
mode, native snapshot fields and 64 KiB reads: identity, length, write/change time
and digest stayed stable. Only access time changed; production already excludes
that field. The build EXE's recorded ChangeTime (12:18:08.635768Z) coincides with
the first refusal's logged second. This is a correlation, not evidence of the
specific mutator; no failure-time before/after snapshots were captured. Probe
records are retained in `executable-snapshots.jsonl`.

The two final startup snapshots were visually inspected: both panels, drive bars,
metadata headers, command line and F1–F12/Jobs toolbar render without an error
banner. These checks establish startup/rendering, not frame cadence, multilingual
software-renderer support, or interactive behavior of every installed plugin.
Evidence is retained under `_build/qa/host-integration-20261112/`:
`final-build.jsonl`, `deployment.log`, `me.sha256`, `runtime-path.txt`,
`final-smoke-{backend,native,software}.log`, `final-native.png`, and
`final-software.png`.
