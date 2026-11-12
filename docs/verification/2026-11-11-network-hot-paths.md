# Typed Network Hot Paths, 11 November 2026

This change completes the discovery-bookmark portion of NET-01. Interactive Linux
authentication remains separate. No LAN discovery, share connection, credential
prompt, or live SMB operation is performed by these tests.

## Contract

- `Bookmark.network` carries the existing `NetworkAddress`. Its provider ID and
  opaque bytes determine identity; the display label and native return folder do
  not. Native, archive and Network locations remain distinct.
- Schema 29 reuses the version-1 Network address codec already used by tabs.
  The strict schema accepts no credential fields; providers remain responsible
  for the credential-free opaque-address contract. The codec performs no caption,
  URI, or UNC interpretation. Existing settings and both values of copy checksum
  verification round-trip without being reset.
- Reopening uses `NetworkAction::Browse` and the restored-tab location machinery.
  It does not activate a share. Missing-provider and stale-address replies keep
  the typed location unavailable; a newer generation rejects an old reply.
- Pending discovery intents are not saved. Queue rejection preserves the current
  pane. Re-adding an address retains its bookmark identity/name/slot, and distinct
  addresses with equal captions keep separate identities. Slot reassignment keeps
  one owner per numbered shortcut.

## Focused evidence

The red gate used the application dependency feature graph, the pinned local
Windows toolchain, `--locked --offline --target-dir _build`, debug information off,
incremental compilation off, two compiler jobs, and an isolated process-local
TEMP/TMP directory. It launched no native application or discovery provider.

The initial gate compiled successfully and produced nine expected failures plus
two passing boundary tests:

| Target | Initial result | Observed missing behavior |
| --- | --- | --- |
| Core `network_bookmark_identity` | 3 failed | Network identity and address validation were ignored |
| Platform `network_bookmarks` | 2 failed, 1 passed | Network address omitted from settings; distinct addresses treated as one native path |
| UI integration `network_bookmarks` | 1 failed | Restored shortcut followed native navigation instead of typed Browse |
| UI presenter `network_bookmarks` | 3 failed, 1 passed | Saving a published discovery address was refused; pending-intent refusal already worked |

Logs: `_build/qa/network-bookmarks-20261111/red-integration.log` and
`red-presenter.log`; the isolated temp directory is recorded in `red-temp.txt`.

The completed green gate passed **49 tests with zero failures**: 25 integration
cases and 24 library cases, including all 11 new Network bookmark regressions.
The two commands built in 36.57 s and 41.60 s respectively. The same pinned
application feature graph and isolated process-local TEMP/TMP setup were used.

```text
cargo test -p midnight-core -p midnight-platform-common -p midnight-ui -p midnight-explorer
  --test network_bookmark_identity --test network_bookmarks
  --test archive_bookmark_identity --test archive_bookmarks
  --test archive_custom_extension_preferences --test network_tab_preferences
  --test tabs_preferences --test transfer_preferences --test preferences
  --no-fail-fast --locked --offline --target-dir _build -- --test-threads=1

cargo test -p midnight-core -p midnight-platform-common -p midnight-ui -p midnight-explorer
  --lib bookmarks --no-fail-fast --locked --offline --target-dir _build
  -- --test-threads=1
```

The commands above are line-wrapped for readability. Both exited zero. Logs:
`_build/qa/network-bookmarks-20261111/green-integration.log` and
`green-presenter.log`; `green-temp.txt` records the isolated temp directory.
Existing native/archive bookmarks, tab persistence, checksum verification and
bookmark save/reconciliation tests passed alongside the new Network cases.
The broader shared Windows and Linux gates are recorded separately by the parent
integration task; these results do not claim that those gates have already run.

## Qualification limits

Persistence tests encode and decode actual preferences bytes. Presenter tests load
those typed preference structures as a new session and inject unavailable/stale
provider outcomes. The UI integration test uses real installed shortcut routing
for each pane with the provider unavailable. These boundaries do not constitute a
restart of a deployed native application against a real SMB/GVfs service. No new
Linux authentication or live-network interoperability claim is made.

## Integrated qualification

The [combined batch record](2026-11-11-queue-batch.md) records final Windows
and Linux gates, strict checks, the matching debug build/runtime, and native/software
startup verification. It supersedes pending integration notes above.
