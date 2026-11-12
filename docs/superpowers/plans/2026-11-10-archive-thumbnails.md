# Archive thumbnails

Continue MEDIA-01 from the accepted provider and thumbnail design. The existing
decoder protocol, isolated helpers, retained Slint images and persistent cache
already support arbitrary readable sources. Connect archive member leases to
these services rather than introducing archive-specific decoders or native paths
constructed from display names.

1. Add an optional archive mount cache-identity capability. Cross-session reuse
   requires the same raw container identity/revision, provider implementation,
   format and deterministic member ID. Existing manifests remain compatible;
   providers must explicitly promise stable IDs before receiving persistent
   preview authority. Duplicate member captions must never share identities.
2. Add an archive source adapter on backend workers. Reject unreadable members,
   links, foreign IDs, unknown lengths and changed revisions; validate bounded
   reads and returned lengths. Keep the current private archive stream staging.
   Scrolling cancels new decoder work/results, never the shared archive mount:
   an admitted read drains under the mount watchdog and retains its real permit.
3. Replace path-only visible thumbnail targets with typed native/archive content.
   UI only snapshots cached metadata; source classification, capture and decoding
   remain on backend workers. Archive previews classify the actual container,
   rather than inheriting storage hints from the previous panel location. Network
   and unknown backing storage require the user's existing network-preview opt-in.
   Archive reads conservatively use the non-SSD I/O budget.
4. Preserve selected/hovered video scheduling, visible quotas, stale generation
   checks, retained pixels on scrolling, resolution-aware cache lookup and clear.
5. Verify adapter/cache/cancellation cases, UI admission without I/O, the real
   application bridge and installed archive/image helper composition. Update
   implementation status with the actual qualification achieved.

Work ownership: a backend agent owns the core mount capability, archive package
advertisement and thumbnail adapter/tests; the main agent owns UI snapshots,
composition, integration and this record. A separate docs agent reconciles stale
Hot Paths/Find/archive/tree status without changing MEDIA-01.

This step does not add writable archives, nested archives or readable remote
provider capabilities. Native network-discovery nodes remain non-content entries.
No third-party dependencies or licensing choices change.

## Completed scope and verification

The five integration steps above are implemented. Windows host suites passed
66 tests; the complete UI library passed 673 tests; the application thumbnail
bridge passed nine tests with one existing native opt-in test ignored. The
independent archive plugin passed 37 tests with three opt-in tests ignored.
Affected strict Clippy checks and the debug application/probe builds passed.
Existing warnings in unrelated engine tree-action code remain.

The actual registered Windows runtime passed the archive image/video probe:
exact image pixels, retained-view restoration without member rereads, persistent
cache reuse after reopening the mount, source-change rejection, new pixels after
mutation, and six different video frames. The registered debug application also
started successfully. See the [verification report](../../verification/2026-11-10-archive-thumbnails.md)
for hashes, logs and the fixture-only endpoint metadata stabilization procedure.

Physical panel input, native Linux execution, persistent cache qualification for
every archive format, large-content performance and release qualification remain.
An admitted archive read retains its real permit until it drains; cancellation
does not terminate shared archive mounts. Generic remote previews and cooperative
per-operation archive stream cancellation remain separate backlog work.
