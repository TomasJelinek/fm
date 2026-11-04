# Implementation ledger — commander controls

- Scope selected by user: external system terminal; read-only controls now,
  mutation operations and their queue remain later. Bottom unsupported actions
  must be disabled rather than bypassing mandatory verification/safety.
- User additionally requires native AltEnter properties, per-pane drive/special
  location buttons, Windows-only Shift+drive-letter and smooth/inertial scrolling.
- Salamander reference audit completed and recorded in design trace.
- Ruling: use PowerShell literals on Windows and POSIX literals on Linux. Command
  execution is explicit external-shell behavior; default terminal stays interactive.
- Ruling: retain Slint's own wheel/touch inertia; custom content-y animations would
  conflict with its native handling and virtualization.
- File ownership: root core contracts/engine/app/docs; native worker platform
  desktop modules; UI worker fm-ui only. FileSystem contracts remain unchanged.
- Added user-requested Ctrl+Shift+S cancellable active-panel folder-size scan and
  active-pane accent/frame to this scope; reference `advwork_calcsize.htm` audited.
- Core RED3→GREEN3 for Unicode literal quoting and bounded RGBA. Engine desktop
  integration GREEN2 verifies modal-properties/navigation isolation and exact
  terminal request capture without executing commands. Backend suite passes.
- Walker RED7→GREEN12 covers Unicode, links, missing/access failures, overflow,
  cancellation, progress throttle and bounded frontier without per-path ceiling.
- Expanded walker GREEN13 and orchestration GREEN11 include queued-but-unaccepted
  listing rollback and stable completed-folder sorts. Snapshot acknowledgement
  separates actual UI state from worker completion; bounded retry is covered.
- User added column asc/desc toggles and automatic filesystem event refresh.
  WatchService is implemented separately on Windows/Linux with notify8.2.1.
  Watch controller GREEN14 includes 1s/3s debounce,15s polling, stale generations,
  full queues, cleanup and unknown-to-known metadata recovery.
- Independent review corrections: Linux FIFO read fixed with O_NONBLOCK and
  opened-file type check; UI Properties uses indexes; shell insertion rejects
  ambiguous quote context; rollback retains actual accepted snapshots.
- Final UI GREEN42 + actual keyboard/render/wheel/automatic-refresh integration.
  Native Windows GREEN8 / Linux GREEN13. Final workspace checks running.
- User extended scope: headerless UI, Blue/Dark/Light, Desktop/Touch metrics,
  persistent preferences, Windows PS/cmd and POSIX Terminal/sudo picker.
- Appearance/picker UI GREEN52 + two actual editor/render binaries; grammar
  binding fixes a reviewed PS-to-cmd injection/late-load preference race.
- Native settings use strict versioned codec, exclusive-temp atomic publish,
  opened-file validation, private Unix modes; graceful latest save is bounded.
  Settings worker GREEN9, common codec/storage GREEN6.
- Storage-aware sizing GREEN24 (13 traversal +11 scheduling tests); global
  weighted budget permits4 SSD reads or1 HDD/network/unknown stream across panes.
  Parent-linked cancellation fixed three cancellation-observation regressions.
- Windows terminal creation now uses owned native processes/new console standard
  handles, preventing command output from leaking to redirected FM diagnostics.
  App/test manifests declare asInvoker/longPathAware; Windows can still reject
  long process cwd explicitly, with no truncation/fallback. Basic FS long paths pass.
- Viewer/VFS and safe reusable-sudo sessions are design-only follow-ups, explicitly
  separated from implemented controls. Main design includes archive/Registry
  providers, per-OS native migration and overwrite metadata/conditional-publication.
- Final Windows/Linux workspace verification is running after these final changes.
- Final verification completed: Windows189 / Linux183 passed; strict workspace
  Clippy, app builds, formatting, source-size85 files and executable smokes pass.
  Native Windows screenshot inspected and real-user FM window is visible/responding
  (PID73472); foreground policy denied stealing focus. Audit0 known matches,
  5 unmaintained transitive warnings. No further code/test work remains in this scope.
