# Windows enumeration metadata

The Windows provider now uses metadata cached by `std::fs::DirEntry` during
enumeration. The backend skips the second per-path metadata pass only when the
provider declares that entry's display metadata complete. Linux and providers
without this capability retain their existing metadata pass. Operation-time
identity checks are unchanged: listing data never authorizes mutations.

The platform contract follows [Rust's DirEntry documentation](https://doc.rust-lang.org/std/fs/struct.DirEntry.html#method.metadata):
on Windows this metadata requires no additional OS call and does not follow a
symlink. Windows metadata translation reuses the same native attribute,
compression and locale/time formatting code as explicit metadata queries.

Local debug measurement, 2026-11-07: 4,000 regular files, seven consecutive warm
runs, enumeration plus complete display metadata; fixture creation excluded.
`midnight-platform-windows --test listing_metadata -- --include-ignored --nocapture`.

| Version | Median | Run range |
| --- | ---: | ---: |
| Separate per-path metadata pass | 75.007 ms | 71.935–80.641 ms |
| Cached enumeration metadata | 20.035 ms | 19.081–23.609 ms |

About 73% less time in this local metadata phase. This is not an end-to-end panel
or network benchmark; sorting, painting and icon loading are outside the timing.
Evidence: `_build/qa/listing-before.log` and `_build/qa/listing-after.log`.
The native metadata regression first failed with empty listing metadata, then
passed. An engine integration test verifies no redundant metadata read/batch and
retained values. Larger drive toolbar icons remain within existing control sizes;
the existing panel geometry integration test passes for mouse and touch density.
