# Worker SDK media consumer verification, 9 November 2026

The four independent media consumers pass their existing test suites on native
Windows x64 and Ubuntu 24.04 Linux x86-64 after upgrading the shared worker SDK.
All checks use the immutable offline bundle
`_build/archive-streaming-api/sdk/worker-sdk-1794226812562261700/.cargo/registry.toml`.
No third-party version, checksum, feature, codec or license alternative changed.

## Native consumer tests

| Consumer | Public SDK | Windows passed | Linux passed |
| --- | --- | ---: | ---: |
| `midnight-viewer-image` | image-viewer 0.3.0, worker 0.2.0 | 10 | 10 |
| `midnight-viewer-pdf` | image-viewer 0.3.0, worker 0.2.0 | 6 | 6 |
| `midnight-thumbnail-image` | thumbnail 0.2.0, worker 0.2.0 | 3 | 3 |
| `midnight-thumbnail-video` | thumbnail 0.2.0, worker 0.2.0 | 2 | 2 |

All suites completed without failed or ignored tests. Image tests cover actual
pixels, alpha, orientation, ICC handling, bounded broker reads and retained
rendering. PDF tests cover source budgets, malformed documents, page geometry,
multiple pages and embedded fonts. Thumbnail tests include real image/video
decoding and malformed/playlist rejection.

For each consumer, the executed command was:

```text
cargo test --manifest-path plugins/<consumer>/Cargo.toml --config <absolute-sdk-config> --locked --offline
```

Windows used `.tools/cargo/bin/cargo.exe`, `CARGO_HOME=.tools/cargo` and
`RUSTUP_HOME=.tools/rustup`. Linux used the native toolchain configured by
`_build/qa/interaction/linux-verification-env.sh` under WSL Ubuntu 24.04.
Both set `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0` and dev/test debug information
to zero. Temporary files stayed under `_build/`; no shared application target
directory was used.

| Consumer | Windows target directory | Linux target directory |
| --- | --- | --- |
| Image viewer | `_build/image-viewer/plugin-tests` | `_build/image-viewer/linux` |
| PDF viewer | `_build/pdf-plugin-target` | `_build/pdf-linux-target` |
| Image thumbnail | `_build/media-thumbnails/windows-image` | `_build/media-thumbnails/linux-image` |
| Video thumbnail | `_build/media-thumbnails/windows-video` | `_build/media-thumbnails/linux-video` |

Video tests reused the verified FFmpeg 9.0.2 prefixes in
`_build/media-thumbnails/native-{windows,linux}/prefix` and libclang 18.1.1 under
`.tools/media-native/`. Linux retained its GCC builtin-header argument and
`-C link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN`. The source archive SHA-256
was rechecked as
`8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`.
Both native configurations still disable GPL, nonfree, version3 and networking.

Commands and complete test output are retained in
`_build/qa/worker-sdk-update/verify-windows.ps1`, `verify-linux.sh` and the eight
`<consumer>-{windows,linux}-test.log` files in that directory.

## Dependency and notice audit

Exact parsed external lock records, including dependency edges and checksums,
match the before-copies under `_build/qa/worker-sdk-update/`:

| Lockfile | Unchanged external records |
| --- | ---: |
| Application | 584 |
| Image-viewer API workspace | 3 |
| Thumbnail API workspace | 3 |
| Image viewer | 38 |
| PDF viewer | 83 |
| Image thumbnail | 38 |
| Video thumbnail | 31 |

Native Windows and Linux metadata/tree captures were refreshed for `app`,
`filesystem-api`, `worker-api`, `image-viewer-api`, `thumbnail-api`, `image-viewer`,
`pdf-viewer`, `archive`, `thumbnail-image` and `thumbnail-video`. Their external
resolve-node feature selections match the previous captures. The eighteen other
capture pairs were retained. No cross-target Windows query substitutes for a
native Linux capture.

```text
cargo metadata --locked --offline --format-version 1 --manifest-path <manifest> --filter-platform <native-target> [--config <absolute-sdk-config>]
cargo tree --locked --offline --workspace --manifest-path <manifest> --target <native-target> --edges normal,build,dev --prefix none [--config <absolute-sdk-config>]
python3 -B scripts/dependency-inventory.py --metadata-dir _build/qa/dependency-inventory
python3 -B scripts/dependency-inventory.py --metadata-dir _build/qa/dependency-inventory --check
```

Both inventory commands passed: **490 package/version/source records, 458 crate
names, 62 direct records**. All three generated inventory files are byte-identical
to their pre-refresh versions. The exact pinned windows-sys/libc MIT notices and
manifest declarations were rechecked; their existing selected alternatives remain.

Image/PDF retained notices cover both native graphs and preserve their upstream
license files verbatim. Existing thumbnail notice staging had only Windows graph
records: it lacked Linux-only libc for the image provider and cfg-if for video.
The regenerated union now retains 36 image and 29 video dependency records, their
full license files and existing Rust notices in
`_build/media-thumbnails/notices/`. The existing ffmpeg-sys-next WTFPL override and
provenance remain; no upstream license file is falsely claimed. These generated
notice directories must accompany any subsequent thumbnail distribution.

Machine-readable evidence is in `external-lock-audit.json`,
`external-features-audit.json`, `notices-audit.json` and
`thumbnail-staging-audit.json` under `_build/qa/worker-sdk-update/`.

## FFmpeg packaging regression

The native build script and both verified prefixes emit `build-options.txt`, but
the release validator required `configure-options.txt`. A focused regression
reproduced rejection of an otherwise complete bundle, exiting with failure before
the fix. The validator now requires the authoritative `build-options.txt`.
The regression still rejects missing libraries, source or build options and
checks exact byte preservation of the options in the copied package.

```text
python3 -B -m unittest scripts.tests.test_worker_packages.WorkerPackages.test_ffmpeg_runtime_requires_complete_libraries_corresponding_source_and_build_notices -v
python3 -B -m unittest discover -s scripts/tests -v
```

The full Python suite reported **67 cases: 56 passed and 11 Windows-only skips** on Linux:
the private deployment-hook test and five native-filesystem/five network-filesystem
Windows build-helper cases. The complete log is `python-tests.log` in the evidence
directory. No other test was skipped or failed.

Real Windows and Linux thumbnail staging uses the freshly tested helpers, all
four existing FFmpeg libraries, the matching source archive and native build
notices, producing 155 retained files for each target. The package-copy check
preserves the source archive and build-options
bytes exactly; evidence and staging remain under `_build/qa/worker-sdk-update/`.
No native runtime or source was downloaded, and the existing FFmpeg builds were reused.

## Limits

This verifies independent consumer tests, dependency selection, notice coverage
and thumbnail release copying. It does not establish a full application release,
fresh installed-host sandbox qualification, GUI behavior, or additional codec
support. The archive provider/host's streaming checks are a separate workstream.
