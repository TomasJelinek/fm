# Custom Command executable artwork implementation plan

> **For agentic workers:** Use superpowers:executing-plans for this approved bounded implementation.

**Goal:** When a visible Program command has no explicit custom image, show its executable's native
system artwork, falling back to the existing Run icon when unavailable.

**Architecture:** Reuse the desktop icon actor, with a coalesced current command-artwork view. Keep
requests in the existing 256-total thumbnail/artwork target budget and existing presentation cache.
Native capture, OS lookup and revision revalidation happen inside the actor's existing guarded
blocking work; only bounded RGBA and metadata cross back to UI. Add no native thread or dependency.

**Tech Stack:** Existing Rust/Tokio/Slint contracts, Windows Shell extraction, bounded Linux desktop
entry/theme lookup using existing PNG loader.

**Spec:** `docs/superpowers/specs/2026-11-07-user-commands-design.md`, Icons section (already approved).

## Constraints

- An explicit custom image has priority even if its decoding fails; never replace that choice with
  automatic artwork. Terminal/script commands retain the generic icon in this scope.
- Stable command ID plus configuration revision identifies images; generation fencing also rejects
  hidden, superseded and cache-cleared replies. Preserve raw paths; never execute a command for icons.
- Maximum 128 configured commands share the existing 256 visible target budget with explicit images
  and panel previews. Coalesce changes, serialize native calls, cap pixels at 64x64 and retain no native
  capture in an async output. Cancellation suppresses obsolete work; OS calls may finish cooperatively.
- Source reads use installed native filesystem capture, then validate before and after OS artwork
  lookup. No direct-path fallback if capture fails. No source/authority or plugin protocol expansion.
- Windows/Linux implementations stay separate. No global engine lifecycle edits by this owner;
  parent integrates the constructor capture argument and one command-dispatch arm.

## File responsibilities

- Core `desktop.rs`/`protocol.rs`: optional defaulted native-icon trait method and metadata-only view/
  result messages. No change to ordinary file-type keys or report-arena contracts.
- Engine `desktop.rs` plus focused icon helper/tests: extend existing actor with current-view
  cancellation, fixed bounds, native capture/revalidation and existing TaskLifetime ownership.
- UI `user_command_presentation*`, `thumbnail_content.rs`, `thumbnail_presentation.rs`, desktop event
  routing: explicit/automatic choice, shared target admission, generation fencing and pixel admission.
- App `thumbnails.rs`/`thumbnail_content.rs` plus one `main.rs` call argument: submit the current
  automatic view from the existing 100ms bridge and exclude automatic rows from image decoding.
- Platform `icons.rs` and focused executable helper/tests: native extraction and bounded theme lookup.
- Dedicated verification note records exact checks, native platform limits and Salamander references.

## Tasks and tests

- [x] Add targeted UI regression for absent explicit image on Program versus explicit/Terminal cases;
  preserve visible-only admission,128commands/256total budget and stale callback behavior.
- [x] Add core contract/helper and actor tests for denied capture, revision change, cancellation,
  out-of-order generations, bounded admission, and no publication of unvalidated/oversized pixels.
- [x] Implement native Windows extraction using documented SHDefExtractIconW and existing RGBA
  conversion/RAII; qualify actual executable artwork and Unicode/native-path validation.
- [x] Implement bounded Linux .desktop matching without executing or interpolating Exec/TryExec,
  then reuse trusted theme PNG loading; cover ambiguous/malformed entries and raw-path handling.
- [x] Wire existing actor/bridge after coordinated shared-file handoff; no redundant thread.
- [x] Run targeted tests under explicit Windows/Linux Cargo leases; independent review and final
  diff/style checks. No commits or broad unrelated tests.

## Reference and limits

Salamander `src/salamdr3.cpp` (`CUserMenuItem::GetIconHandle` and background icon reader) provides a
behavior reference: explicit icon first, native command icon second, generic fallback, and current-menu
version fencing. Its implementation is not copied; fixed ANSI/MAX_PATH and icon-index parsing are not
adopted. Windows uses the documented default extraction API, not PrivateExtractIconsW (Microsoft says
that API is not intended for general use). Linux preserves the existing bounded PNG theme policy;
missing/unmatched or SVG-only artwork can legitimately leave the generic Run icon.
