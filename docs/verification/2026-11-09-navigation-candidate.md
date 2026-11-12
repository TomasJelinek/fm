# Navigation candidate deployment and native smoke — 2026-11-09

The combined debug candidate is installed privately at
`C:\Users\tjelinek\MidnightExplorer-20261109-navigation-candidate\me.exe`.
Its native startup smoke test passed with exit code 0 and empty stderr. Both
panels rendered their requested directories, and the captured drive bars show
Network in the same relative position.

## Build and deployment provenance

- Root build session 36254 completed with exit code 0 after 631 UI tests and
  strict UI/app all-target Clippy with `--no-deps -- -D warnings`. The build log
  is `_build/qa/custom-command-preview/build.log`.
- Executable: 86,073,856 bytes, SHA-256
  `69A4A2C5A2E81DC93B82C74866594F836770E8DBEDACCDD974CDD8A490C2DFD1`.
- The new directory contains 580 files: the approved executable and 579 verified
  plugin/notice files copied from the existing panels candidate. Deployment
  verified the installed file hashes and the new directory's protected private
  DACL using the existing `scripts/deploy-private.ps1` policy.
- Native Windows plugin SHA-256:
  `21E82F4DEA4588E06D2B6F372DFE54AAE3527D454825A949F37B96C08FCA9EAC`.
- Archive worker SHA-256:
  `069B835F2CFE37150137BBCC14DD38D9E8CD0E2F6DCDADFAAB0A8C600347C922`.
- The frozen panels candidate's three root executables and root ACL remained
  unchanged. No shortcut was created, runtime registered, or ancestor ACL
  changed.

The prepared action and its readback evidence are under
`_build/qa/navigation-candidate/`: `prepare.ps1`, `deploy-confirmed.ps1`,
`plugin-inventory.json`, `frozen-baseline.json`, `deployment.json`, and
`deploy.log`. Deployment ran only after root confirmed the successful build and
the exact executable hash.

## Native smoke result

The owned candidate process ran with these arguments:

```text
--ui-smoke-test --left D:\experiments\dev\FM --right C:\Windows\System32 --snapshot D:\experiments\dev\FM\_build\qa\navigation-candidate\native-smoke.png
```

The default native renderer was used. Process 66228 ran from
15:01:09.641 to 15:01:29.248 UTC, exited normally with code 0, and did not reach
the timeout. Stdout contained only the snapshot path; stderr was empty. Current
and legacy preference file hashes/existence were unchanged. Smoke mode uses
in-memory preferences and disables window-state persistence.

The 3015 × 1845 capture was visually inspected: both directory listings and
compact drive buttons were visible, with no startup error or dialog.

Evidence under `_build/qa/navigation-candidate/`:

- `native-smoke.json`: executable hash, owned process, arguments, exit result,
  and preference comparison.
- `native-smoke.out`, `native-smoke.err`, `smoke-run.log`: process output.
- `native-smoke.png`: native window capture.
- `preferences-before.json`, `preferences-after.json`: preference provenance.
- `smoke.ps1`: bounded owned-process smoke runner.

This verifies native startup and the captured navigation presentation. It does
not replace each plugin's format tests or a manual archive-size workflow. The
manual archive-size workflow remains pending separately. Network ordering and
activation regression evidence is recorded in
`2026-11-09-navigation-ordering.md`.
