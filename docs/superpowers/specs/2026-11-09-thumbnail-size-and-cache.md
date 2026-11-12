# Configurable thumbnail size and persistent cache layout

Options → Previews offers thumbnail image edges of 48, 64, 72, 96, 128, 192 and
256 logical pixels. The default is 96 (updated 11 November); saved explicit sizes remain unchanged.
The setting is shared by both panels, saved with OK, reverted with Cancel and
included in Reset to defaults. It changes thumbnail tiles, not detailed-list row
height or toolbar artwork. Text/control sizing continues to follow font and density.

Display scale determines the required physical resolution. The existing decoder
API supports bounded images up to 512 pixels; no plugin ABI or source-file size
limit is introduced. Backend resolution buckets retain the existing 96-pixel
cache variant and add 128, 192, 256, 384 and 512. A larger cached decode can cover
a smaller request. Coverage records the requested decode resolution independently
of actual bitmap dimensions: an intrinsically tiny source must not be decoded
repeatedly in an attempt to make it bigger.

Changing size never clears the whole disk or memory cache. Insufficient resolution
is a miss for that file/frame only. Visible items are upgraded on bounded backend
workers. A previously displayed smaller image remains visible until a replacement
is ready; late smaller results cannot downgrade it. Stable entry identity, source
revision and decoder identity remain separate from resolution. Source changes
invalidate their own cached artifacts through the existing key validation.

Decoded pixel budgets still apply. Replacement accounting subtracts the old frame,
and pressure must not evict visible fallback images merely to admit their larger
replacements. Source reads, decoding, cache lookup, checksum verification and disk
writes remain outside the UI thread. Size changes do not bypass storage/network
concurrency policy or cancel real worker permits prematurely.

Current limits are 54 MiB of retained UI pixels (at most 4,096 image records),
64 MiB for the backend memory cache and 512 MiB / 8,192 physical disk records.
These are separate cache budgets, not a whole-process or GPU-memory limit.

If the UI cannot admit a decoded upgrade within its pixel budget, it retains
only bounded retry metadata. A changed viewport or size retries that frame using
the same captured source and the disk cache. It does not continuously decode or
upload while the view is unchanged. Actual retained coverage distinguishes an
accepted tiny-source result from a refused upload; stale retries cannot reduce
newer coverage.

Disk cache paths remain:

- Windows: `%LOCALAPPDATA%\Midnight Explorer\cache\thumbnails\v1`.
- Linux: `$XDG_CACHE_HOME/midnight-explorer/thumbnails/v1`, falling back to
  `~/.cache/midnight-explorer/thumbnails/v1`.

New artifacts use two hexadecimal hash characters as a shard directory:
`v1/ab/<64-character-hash>.mtc`. This bounds the namespace to 256 buckets without
creating every directory up front. Existing flat `v1/<hash>.mtc` records remain
readable; there is no bulk migration or forced regeneration. New writes replace
their legacy counterpart. Enumeration, eviction and Clear thumbnail cache cover
both layouts with one aggregate budget and one root transaction/generation.
Duplicate physical records must count toward both byte and record limits.

Shard paths are derived solely from validated hash basenames. Native storage code
opens private child directories through existing pinned parents, rejects symlinks
and Windows reparse points, and retains atomic writes and record checksums. It must
not turn cache sharding into arbitrary path traversal or weaken ACL validation.

Tests cover settings migration/roundtrip, OK/Cancel/Reset, geometry at different
sizes, retained smaller frames during upgrades, intrinsic tiny sources, stale
completions, cached larger-frame reuse without content reads, sharded/legacy reads,
aggregate eviction/clear and native path confinement on Windows and Linux.
