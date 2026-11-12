# Native Custom Commands from inline trees

## Current gap and scope

At authoritative baseline `1c1b0b6`, Ctrl+F8 already uses typed native tree captures, common checksum calculation, worker retirement and actual-window tests. Do not duplicate that completed consumer. Custom Commands still enter `request_selection_action`, which deliberately rejects adopted trees.

Implement native Custom Commands through the existing bounded command actor and process preparation. The contract distinguishes captured selected occurrences from the focused occurrence: Once commands use focused file variables; Per selected item and grouped argument placeholders use marked entries, with focused fallback only when unmarked. Active/other panel-directory authority remains the existing location context, separate from file arguments. Literal Once commands retain their no-file behavior at parent/no-focus.

## Changes

1. Add an immutable core tree-command request and opaque captured native source contract. Retain exact projection, selection revision, request, cancellation and provider ownership; reject parent/context/unsupported selected occurrences and archive-to-native fallbacks.
2. Reuse tree `capture_selection` / `capture_target` and the shared four-capture budget. Selected and focused targets are independently captured on the tree worker. Native argument entries are reconstructed from provider-resolved paths and retained kind/metadata, never from display captions or an assumed native tree row path.
3. Feed the opaque source into existing command preparation on its worker. Reuse template expansion, budgets, native pins, preview and process launch. Preserve captured source ownership and revalidate it with the existing pins before actual launches. Preview performs no launch. Disposal and rejected transport cleanup stay in the existing lifetime domain.
4. Add a presenter consumer reusing `tree_capture` acknowledgement/pending-selection handling. Capture command configuration and Settings preview origin once. Refuse late navigation/projection/settings receipts; preserve queue refusal semantics and cancellation.
5. Add focused core/backend/presenter and actual Rust/Slint tests, then run coordinated targeted checks. Windows Cargo is reserved by the scroll owner; coordinate Linux Cargo with the Find owner. No image/viewer/scroll edits, no commits.

## Tests and review

Cover selected/focused distinction, native descendants and folders, opaque-name/path preservation, context/parent/archive/mixed rejection, exact acknowledged identities, stale and duplicate receipts, queue pressure, cancellation, provider revocation, changed native source kind/metadata, source ownership through a blocked native launch and worker destruction, literal Once commands without a file, and Settings preview retirement. Root reviews integration independently. Record actual executed evidence and outstanding native qualification separately.

## Completed checkpoint

All five implementation steps are complete for native Custom Commands. Windows
and Linux targeted gates, actual native-process integrations, final early-rejection
ownership regressions and independent review are recorded in
[tree command qualification](../../verification/2026-11-12-tree-custom-commands.md).
The fresh Windows debug deployment and startup observations are recorded in
[host integration](../../verification/2026-11-12-tree-images-debug.md).
Archive exports and other guarded TREE-01 consumers remain separate backlog work.
