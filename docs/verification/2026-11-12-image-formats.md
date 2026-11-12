# Shared image formats qualification

Final result: 34 affected tests plus strict Clippy per platform, and nine actual
production-package restricted-host cases per platform, all pass for shared SVG
and AVIF viewing/thumbnails. HEIC/HEIF is not enabled. The evidence below retains
the earlier checkpoints and the actual failures that led to the final guards.

The initial independent workers rejected real SVG content as Unknown format:
viewer 2 tests and thumbnail 1 test failed before decoder changes. The evidence is
`_build/qa/image-formats-20261112/svg-viewer-red.log` and
`svg-thumbnail-red.log`. Both failures exercised the production decode entry.

The shared SVG safety suite then exposed an XML namespace-reset case that accepted
an image element as blank content. Its RED is retained in `svg-namespace-red.log`
and `svg-resource-diagnostic.log`; the guard now handles absent and empty namespace
representations before SVG conversion. The actual image resolvers remain disabled.

Linux targeted GREEN: shared decoder 6, image viewer 13 and thumbnail worker 4,
23 tests total. The worker suites retain existing PNG/JPEG/GIF, orientation,
ICC/alpha, viewport and read-budget checks. New tests check SVG original pixels,
thumbnail aspect/alpha, distinct real Arabic/Hebrew glyph fallback, local gradients
and reuse, unsupported resources, DTD/depth/reference expansion, excessive original
dimensions and bounded reads of unknown input. Logs are `image-decoder-green.log`,
`midnight-viewer-image-green.log` and `midnight-thumbnail-image-green.log` in the
same QA directory. Shared decoder strict all-target Clippy passed; final worker
strict checks and package validation are recorded separately when complete.

An independent read-only review found no actionable source/resource authority,
expansion, pixel-budget, alpha or bundled-font issue. SVG is explicitly a static
subset; unsupported images/effects fail instead of silently claiming full support.
No installed-worker or Windows result is inferred from the Linux library tests.
No AVIF or HEIC support is claimed at this SVG checkpoint.

The shared decoder and both workers pass strict all-target Linux Clippy. Initial
Clippy identified only a stale extracted import and fixed-array iterator style in
new tests; those were corrected without changing assertions. The final logs are
`image-decoder-clippy.log`, `midnight-viewer-image-clippy.log` and
`midnight-thumbnail-image-clippy.log`. Both independent provider locks now record
version 0.2.0. Inventory captures for all three independent workspaces and both
platforms are fresh; generation and `--check` pass with 625 package/version records,
512 names and 64 direct records. No new third-party version was introduced.
The shared notice set retains actual files for 75 pinned dependency versions plus
the embedded Noto fonts. Both image-worker packaging paths copy this complete set.

The frozen SVG checkpoint also passes all 23 Windows tests through the reusable
runner's new explicit independent-workspace support. Three runner invocations run
concurrently, each with at most 2 processes and a thread budget of 6; actual peaks
are 1 for shared safety and 2 each for viewer/thumbnail. The 23 tests pass with no
cleanup errors. Receipts, exact executable hashes, per-package working directories,
private output and case logs are retained under
`_build/qa/image-formats-20261112/windows-svg-image-decoder/`,
`windows-svg-midnight-viewer-image/` and `windows-svg-midnight-thumbnail-image/`.
This is artifact-level Windows validation, not an installed-worker claim.

The subsequent AVIF phase first reproduced Unsupported(Avif) for real opaque and
color/alpha fixtures (`avif-red-windows.log`). The native constructor uses the
same bounded settings for both planes. Reduced-budget tests prove a real 33x11
primary is rejected, and a container retaining that real alpha bitstream with a
1x1 primary is also rejected by the alpha limit. The opaque fixture was encoded
upstream at default lossy quality, so its near-white channels use a three-level
tolerance; the lossless-quality color/alpha fixture compares against its upstream
PNG reference. Fixture origin, hashes and BSD terms are retained beside the files.

A distinct regression found mp4parse Normal mode could suppress a budget-reader
error while skipping a trailing unknown box. The actual failure is retained in
`avif-encoded-trailing-red.log`. AVIF now stages its limited encoded bytes before
calling the parser and checks one extra byte, so excess cannot become successful
truncation. No source ceiling or complete read was added to other raster formats.

Both Windows and Linux then pass 32 targeted tests (shared decoder 13, viewer 14,
thumbnail 5), plus strict all-target Clippy for all three independent workspaces.
The binary links use pinned static dav1d 1.5.4 on both platforms. Three independent
runner invocations use at most two processes and six test threads each; all have
zero failures and cleanup errors. Exact receipts, executable hashes and per-case
logs are under `{windows,linux}-formats-{image-decoder,midnight-viewer-image,
midnight-thumbnail-image}` in the same QA directory. An independent coordinator
review found no blocking issue in input staging, primary/alpha native settings,
resource bounds, vendor patch or consumer regressions. These results do not yet
claim installed-worker qualification; that separate gate follows packaging.

AVIF inventory captures for all three independent workspaces and both platforms
were regenerated and checked: 646 package/version/source records, 528 names and
66 direct records. The shared distribution retains actual notices for 107 pinned
Cargo package versions, bundled OFL font material, native dav1d 1.5.4 COPYING and
source provenance, the image-rs constructor patch note, and the exact 82408-byte
mp4parse 0.17.0 source crate (SHA-256 recorded beside it). The latter satisfies the
source-availability part of distributing the unchanged MPL-covered parser without
changing the host/application license. Logs: `inventory-avif.log`, `notices-avif.log`.

A separate [actual restricted Windows host checkpoint](2026-11-12-image-host-formats.md)
passes seven cases against the source-test worker binaries. It covers SVG pixels,
AVIF color/alpha dimensions, retained zoom without rereads, PNG, cancellation and
malformed/truncated input. Production-packaged hashes still need that gate rerun;
Linux installed-worker qualification is not inferred from native library tests.

Focused build-wrapper fixtures also pass: Windows two tests (one Bash-only skip),
Linux one test (two Windows-only skips). They execute the real wrapper against
inert compiler/plugin fixtures and check image-only offline forwarding, unchanged
PDF parameters, and complete environment restoration on success/failure, including
absent versus empty values introduced by native compiler activation. Logs are
`wrapper-tests-windows.log` and `wrapper-tests-linux.log`.

Final review found a concrete f32 rounding overshoot: 13x13 scaled to a 7x7 target
became 8x8 after ceil. Both shared decode and the actual thumbnail provider failed
the new exact regression before the fix (`svg-sizing-shared-red.log`,
`svg-sizing-thumbnail-red.log`), and the installed host independently rejected that
frame (`host-probe/packaged-svg-rounding-red.log`). Output extents are now clamped
to the requested bounds, preserving coverage, aspect fitting and no upscaling.
The regression also checks a non-square 13x26 source and a larger requested size.

Final affected tests pass on both platforms: shared decoder 14, viewer 14,
thumbnail 6, 34 total per platform, with strict all-target Clippy for all three.
Their new unique runner directories are `{windows,linux}-formats-svg-sizing-*`;
the previous 32-case receipts are retained separately. A runner refusal to reuse
an earlier output directory was resolved by selecting these fresh directories;
no product test failed after the cap correction.

The production Windows build entrypoints then passed with offline pinned native
archives and the isolated verified-Meson launcher. Final manifests advertise
SVG and AVIF, reject HEIC/HEIF, and use thumbnail cache policy r3 including dav1d
1.5.4. Both packages contain 213 exact shared notice/source files. PE imports are
only Windows/system CRT libraries, with no additional codec DLL. Final worker
hashes are recorded in `package-hashes-windows.json`, and content checks in
`package-audit-windows.json`. The final actual-manifest restricted-host gate passes
all nine cases in 8.44 seconds, including exact 13-to-7 pixels/dimensions; see the
[host evidence](2026-11-12-image-host-formats.md). Linux production packaging and
its final matching-hash host gate are recorded separately on completion.

Linux production packaging also completed through the final verified native
bootstrap/helper paths, using the same existing owned build root. The viewer
and thumbnail Rust builds completed in 2m59s and 2m11s respectively. Both shipped
manifests and all 213 notice/source files pass readback audit. ELF imports are
only libgcc_s, libm, libc and the system loader, with no dynamic codec library or
development RPATH. Final hashes are in `package-hashes-linux.txt`; audit and import
records are `package-audit-linux.json` and `*-package-linux-dependencies.log`.

All nine final Linux actual production-package host cases pass in 0.06 seconds,
with matching audited binary hashes. The first QA copy preserved writable DrvFS
modes and was correctly refused by viewer discovery (four thumbnail cases passed,
five viewer/association cases refused). That refusal is retained in
`host-probe/final-packaged-linux-inherited-mode-refusal.*`. Protecting the owned
native Linux installation with directories 700, data files 600 and executables
700 made the same nine tests pass; no product trust check or assertion changed.
Final logs are `host-probe/final-packaged-linux-restricted-worker.{json,log}`.
The [host evidence](2026-11-12-image-host-formats.md) records exact final hashes.

An independent wrapper review found no actionable issue in image-only offline
routing, PDF parameter compatibility, complete Windows environment restoration,
per-image Linux sysroot isolation, shared notices or optional video selection.
Production source, manifests and package hashes are frozen after these gates.
