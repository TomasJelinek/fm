# Midnight archive filesystem worker

Independent read-only executable plugin for ZIP, GZIP, TAR/TAR.GZ and 7z. This
project consumes only versioned public filesystem API and worker SDK packages,
plus pinned open-source format libraries. Its tests create real fixtures locally.
It neither depends on nor writes to application source files.

The helper must be started by the host's restricted-worker launcher. It accepts
no input filename and no command arguments. It completes the public worker SDK
sandbox handshake before accepting archive bytes. A failed sandbox is terminal.
The host supplies bounded source bytes and owns cancellation and revision checks.

Supported content: ZIP store/deflate; GZIP including concatenated members;
TAR regular files and directory metadata (GNU/PAX long names provided by tar-rs);
7z Copy/LZMA/LZMA2 and enabled pure Rust BCJ filters, solid and non-solid. Encrypted,
unsupported codec and multi-volume input is not silently delegated to an external
utility. Links, special entries, traversal, absolute names, backslashes, colons
and NUL names are not readable or extractable. Duplicate member names have
independent opaque IDs. GZIP's untrusted FNAME never controls its synthetic member
name, which is `content`.

The initial mount caches verified content in bounded memory. Limits: 64 MiB input,
64 MiB member, 128 MiB decoded aggregate, 100,000 entries, 8 MiB names, expansion
ratio 200. Exceeding them returns Limit. Solid archives decode serially once per
mount; later member reads use the verified cache. There is no disk extraction,
archive writing, password support, RAR codec or nested mounting in this release.

Build and test using `scripts/build.ps1` or `scripts/build.sh`, with Cargo/Rust and
Python 3 on PATH, providing an offline
public SDK config, a build output directory and an optional package destination.
Source directories are not output destinations. Exporting this directory does not
require the host checkout. See THIRD-PARTY.md for pinned dependency decisions.

Host browsing uses opaque member IDs. Saved tabs store the provider ID, native
container path, captured source revision and raw member components separately.
On restart only active tabs reopen; missing providers and changed sources remain
unavailable read-only tabs with their addresses retained. Display labels never
become native paths. In-session tabs, parent navigation, history, sorting,
filtering and selection operate on provider identity. F3 receives a bounded
content lease; external editing, shell launch, mutations and native clipboard
operations are unavailable for archive members.
