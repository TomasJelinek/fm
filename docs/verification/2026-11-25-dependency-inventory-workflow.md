# Dependency inventory from each host's captures

The tables in `docs/dependencies/` are generated from the Cargo metadata and trees of
the 23 inventory workspaces ([procedure](../third-party.md#regenerating-the-inventory)).
Build dependencies and procedural macros follow the host, so each target is captured on
its own host. A Windows query on Linux is not equivalent: for the application it listed
Linux-only build dependencies of Slint (`dlib`, `yeslogic-fontconfig-sys`) that the
Windows metadata lacks, and the generator refused it.

## Change

- `scripts/capture-dependency-inventory.ps1` captures every scope for the host it runs
  on. `-IncludeMacOS` adds `native-macos` for both Darwin targets after checking that the
  package has no build or procedural-macro dependency.
- The `dependency-inventory` workflow packages the released SDK bundles once, captures on
  `windows-latest` and `ubuntu-24.04` and regenerates the tables. Its jobs keep no Cargo
  download cache. The owner decided on 25 November 2026 that tables which differ from the
  captures fail the run; the `dependency-inventory` artifact holds the regenerated
  tables, their diff and the captures.

## Results

- Run 45703494519 (commit `0ccc198d`, before the check became strict) passed every job:
  SDK bundles 40 s, Windows capture 1 min 36 s, Linux capture 40 s, inventory 25 s, with
  Cargo 1.99.1 and PowerShell 7.6.10 on both runners.
- Its Linux and macOS captures equal local ones taken in the project's build image with
  Cargo 1.99.1: the trees are identical apart from the checkout path, and the generator
  gives identical records.
- The regenerated tables differ from the committed ones in one record:
  `i-slint-renderer-femtovg` 1.18.1, vendored that day, is a local path dependency with
  `midnight-ui-input` as a direct dependant (direct records 80 → 81). Rows that hand
  edits had placed out of the generator's order moved; their content is unchanged. The
  columns that the About dialog reads are identical. `fdd8a369` committed these tables;
  tables generated locally from the run's captures are byte-identical to them.
- [lockfile-only.md](../dependencies/lockfile-only.md) needed no row change: every
  third-party package of the 23 lock files is an inventory record or a row there, never
  both, and each row names exactly the workspaces whose lock files pin it.
- Locally on Linux, the script under PowerShell 7.7.0 gave the same 22 Linux and 2 macOS
  pairs as a Bash capture in the build image. It refused an unknown commit, a changed
  lock file, an output folder holding other files and a checkout nested in another one.
  The workflow's step scripts, read from the YAML, packaged the bundles (Cargo `--locked`
  accepted their checksums), captured with them, and compared stale and current tables.

Not covered: the script on a developer's Windows machine with the bundles that a Windows
build recorded (the workflow passes its own), and the strict check on GitHub, which the
next run of the workflow exercises.
