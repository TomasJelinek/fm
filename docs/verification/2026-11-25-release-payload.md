# Release payload checks for 0.6.0

The locally built Windows and Linux HTML viewer packages used the pinned Chromium
Embedded Framework 156.0.26. Each package contained only `locales/en-US.pak`.
These are uncompressed sizes in bytes.

| Platform | Locale files in the pinned runtime | Locale files shipped | Original locale bytes | Shipped locale bytes |
| --- | ---: | ---: | ---: | ---: |
| Windows x64 | 220 | 1 | 52,217,880 | 604,564 |
| Linux x86-64 | 220 | 1 | 51,281,645 | 593,324 |

The Windows package retained the SDK's byte-identical 294,650,880-byte
`libcef.dll`. The Linux package was built with `build-linux.sh … release` on
Ubuntu 24.04.4. GNU binutils 2.42 stripped four staged files after SDK
verification. Their complete `readelf --dyn-syms -W` outputs matched the
originals. Release inspection found no full symbol table or DWARF in the result.

| Linux file | Verified SDK bytes | Shipped bytes |
| --- | ---: | ---: |
| `libcef.so` | 1,480,763,264 | 278,346,904 |
| `libvk_swiftshader.so` | 15,073,248 | 4,025,704 |
| `libvulkan.so.1` | 1,561,800 | 703,944 |
| `chrome-sandbox` | 31,952 | 15,104 |

Both release providers passed the opt-in native
`chromium_renders_captured_html_and_denies_default_scripts` integration test
with `MIDNIGHT_HTML_TEST_EXECUTABLE` pointing into the distributed package.
The test exercises actual rendering, offline/script restrictions, input, Find and
shutdown. Windows took 9.59 seconds. Linux took 3.68 seconds under Xvfb with
`LC_ALL=cs_CZ.UTF-8` and a Czech locale installed. The Linux test observed three
Chromium descendants with namespace isolation, seccomp and NoNewPrivs enabled.

Independent release builds of the Linux native and network filesystem providers
exported `midnight_filesystem_entry_v1`. Neither library had a full ELF symbol
table or DWARF debug info.

The production packager produced an unpublished Windows verification ZIP of
245,982,988 bytes. All 2,373 members passed CRC verification. A second pass
over the ZIP found 27 PE images, one locale pack and no embedded debug symbols
or symbol sidecars. The 76,573,184-byte application has Windows FileVersion and
ProductVersion 0.6.0. The ZIP's SHA-256 is
`131613fd97dfbc8a41ef8fe438b40b393e16adc8a5c68dfa1ab16d70fa0731a9`.

This local ZIP used freshly built application, elevation helper, HTML viewer,
native filesystem and network filesystem binaries. The other five viewers,
archive filesystem and two thumbnailers came from the 0.5.3 Windows release.
Their implementation sources, dependency locks, public SDKs and Rust toolchain
matched the checked-out versions. The packager validated all six viewers, three
filesystems and two thumbnail providers. The Linux native tests used the rebuilt
HTML and filesystem packages.

The following packaging suites passed on their respective platforms.

| Suite | Windows | Linux |
| --- | --- | --- |
| `scripts/tests/test_package_release.py` | 27 passed, 7 skipped | 29 passed, 5 skipped |
| `scripts/tests/test_runtime_packages.py` | 12 passed | 12 passed |
| `scripts/tests/test_html_viewer_build.py` | 18 passed, 6 skipped | 12 passed, 12 skipped |
| `plugins/midnight-viewer-html/tests/test_linux_package.py` | | 9 passed |
