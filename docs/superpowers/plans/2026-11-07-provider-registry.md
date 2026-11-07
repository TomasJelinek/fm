# Provider registry implementation checkpoint

This implements the accepted VFS design incrementally. The current native
services and archive worker remain operational during migration. This checkpoint
does not claim that native filesystems have become independent plugins.

1. Add validated package identities, generation-scoped mount/node references and
   lossless versioned stored addresses. Runtime references never enter settings;
   captions and path separators never determine identity.
2. Add a backend-only registry and Browse/Read lease contracts. Dispatch performs
   no provider work while holding the registry lock. Removal revokes old work;
   independently retained content remains alive until explicit revocation or the
   last consumer releases it. Unsupported services fail explicitly.
3. Exercise independent fake providers with identical node numbers, unavailable
   restores, stale references, foreign nodes, exhaustion, cancellation and blocked
   providers. Check returned sizes and metadata before publishing results.
4. Add transitional native/archive adapters only behind these same contracts;
   migrate production navigation and selection after compatibility tests. Publish
   the independent ABI/SDK before extracting native plugin implementations.

The complete migration, inline tree and network neighborhood remain governed by
the existing VFS/network designs and implementation backlog. No preferences
schema is allocated here; schema 18 belongs to expression filters.

Implemented in this checkpoint: typed identities/addresses, revocable registry,
bounded known-length content leases, transitional native/archive Browse/Read
adapters and focused conformance tests. Production navigation/selection/F3 cutover
and the public generic filesystem ABI remain pending. The native adapter keeps
existing platform services; it does not fulfill independent native plugin delivery.

Verification artifacts belong in `_build/goal/vfs/`. Windows/Linux build slots
are shared with the filter and transfer work. No new dependencies are required.

Ruling: use bounded opaque node tokens, not bounded path strings. Providers map
tokens to their own native or virtual identities. Stored address bytes have no
fixed path-length limit; decoding/enqueueing callers enforce their aggregate
resource budget without truncation. A reference is a routing identity, never
permission to mutate an object or evidence of an immutable filesystem snapshot.
