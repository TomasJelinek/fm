# User viewer associations — 9 November 2026

Options > Plugins now edits literal extensions and priority across native viewers
and isolated image/PDF workers. The user's settings contain overrides; installed
manifests and their trust/capability checks are not changed. Startup snapshots retain
the existing restart-based enablement model. Basic fallback and Alt+F3 keep their
defined behavior, and explicit Source mode is no longer overwritten by binary
sniffing merely because that plugin also exposes Binary.

The settings codec uses schema 27 only when needed. Older schemas inherit manifest
defaults. Missing plugin preferences survive installation changes. Duplicate IDs,
noncanonical suffix lists, excessive settings and invalid literal paths are refused.
Unicode combining characters are accepted as literal suffix text.

## Evidence

Logs are under `_build/qa/make-file-list/`.

- Core normalization/validation/matching: three causal RED tests became GREEN.
- Five backend routing regressions first failed against the stub selector, then
  passed. They cover native/process precedence, compound suffixes, disabling,
  fallback role, Alt+F3 and Source+Binary mode selection.
- Core/common libraries: **47 + 75 tests passed**, including strict storage
  migration, malformed data and roundtrips.
- Main UI: **653 tests passed**. New cases cover draft/Cancel/reset, one OK commit,
  invalid and excessive input, missing providers and an asynchronous catalog race.
- Real widget tests: **3 passed**, covering keyboard list navigation, reordering,
  multiline editing, manifest Reset, Cancel/reopen, validation and successful save.
  Two additional causal failures exposed hidden drafts after catalog refresh and
  stale list ordering during Options previews; both have targeted regressions.
- Independent presenter/UI review found no remaining actionable issue. The
  rendered Plugins page and error state were visually inspected. No new dependency
  or external plugin API/ABI was added.
