# System requirements

Applies to Midnight Explorer **0.2.0**. Rendering policy reviewed on **2026-11-11**.
Build/run commands are in the [README](../README.md#build-and-run).

## Operating systems and support status

These are the project's compatibility targets, not a claim that every listed OS,
desktop, graphics driver or filesystem has passed release qualification.

| Platform | Minimum targeted OS / architecture | Current verification and limitations |
| --- | --- | --- |
| Windows | Windows 10 22H2, **x64**, MSVC build; Windows 11 recommended | Native adapter implemented. Development checks run on Windows 11 Enterprise 25H2, build 26200.9711. Windows 10 still needs a dedicated qualification run. Use an OS edition receiving security updates. Windows 7/8, 32-bit, native ARM64 and MinGW builds are outside the current target. |
| Ubuntu | Ubuntu **24.04 LTS**, **x86-64**, GNU/glibc; newer LTS versions are compatibility targets | Native adapter implemented. CI configuration targets 24.04; local Linux checks use Ubuntu 24.04.4 under WSL2, kernel 6.6.87.2, glibc 2.39. This does not qualify every physical Linux desktop. Ubuntu 26.04 needs its own desktop/renderer validation. |
| Linux Mint | Mint **22.x**, **x86-64**, Ubuntu-based edition | Uses the same Linux adapter and Ubuntu 24.04 library baseline. A native Cinnamon/MATE/Xfce qualification run is still required. LMDE is not separately qualified. |
| Fedora | **Fedora 44** and newer maintained stable desktop releases, **x86-64**, GNU/glibc | Explicit compatibility target using the same Linux adapter. Native build, GNOME/KDE, SELinux-enabled operation and graphics qualification are pending; Rawhide is not a release target. |
| openSUSE | **Leap 16.x** and current **Tumbleweed**, **x86-64**, GNU/glibc | Explicit compatibility targets using the same Linux adapter. Build and desktop/graphics/filesystem qualification are pending. Record the tested Tumbleweed snapshot, not a permanent rolling-release guarantee. SUSE Linux Enterprise requires a separate target/version qualification. |
| Other Linux | x86-64 GNU/glibc desktop with the dependencies below | Source builds may work; no distribution-independent binary guarantee. Older Ubuntu/Mint, musl/Alpine and ARM64 are not current release targets. |
| macOS | **Planned; no supported version or architecture yet** | There is no macOS platform adapter, so the current application does not build or run there. Intel/Apple Silicon versions, deployment target and RAM minimum must be qualified with that implementation. Slint's macOS support alone is insufficient. |

The Windows baseline also satisfies the Rust toolchain's Windows 10-or-newer
requirement ([Rust MSVC platform support](https://doc.rust-lang.org/rustc/platform-support/windows-msvc.html)).
Mint 22 uses the Ubuntu 24.04 base
([Mint 22 release notes](https://www.linuxmint.com/rel_wilma.php)).
openSUSE Leap 16 requires an **x86-64-v2** CPU; this is the OS baseline even though
our release compiler does not use host-specific CPU optimization
([Leap release notes](https://doc.opensuse.org/release-notes/x86_64/openSUSE/Leap/16.0/html/release-notes-leap-160/index.html)).

## RAM and hardware for running the application

The following are **provisional provisioning requirements**, not measured
minimum-memory guarantees. No constrained-RAM acceptance benchmark has established
the application's hard minimum. Total RAM includes the OS; application headroom
means available memory after the desktop and other applications are running.

| Platform | Minimum total RAM to provision | Recommended total RAM | Application headroom to reserve |
| --- | --- | --- | --- |
| Windows 10/11 x64 | **4 GB** | **8 GB or more** | **1 GiB** for ordinary browsing; **2 GiB or more** for large listings and concurrent work |
| Ubuntu 24.04 desktop x86-64 | **4 GB** | **8 GB or more** | Same 1 GiB / 2 GiB provisional allowance; Linux memory benchmark pending |
| Ubuntu 26.04 desktop x86-64 | **6 GB** | **8 GB or more** | Same allowance, in addition to sufficient desktop headroom |
| Mint 22.x x86-64 | **4 GB** for this application's target configuration | **8 GB or more** | Same 1 GiB / 2 GiB provisional allowance; Mint memory benchmark pending |
| Fedora / openSUSE x86-64 | **4 GB or the desktop's minimum, whichever is higher**, as a product planning allowance | **8 GB or more** | Same 1 GiB / 2 GiB provisional allowance; native measurements pending |
| Other Linux | At least the desktop's own requirement and the application headroom above | **8 GB** is a practical starting point | Depends on the desktop and workload; not qualified |
| macOS | **Not established** | To be measured when the adapter exists | Same application memory budget is a design goal, not a supported configuration |

Windows 11 itself requires 4 GB
([Microsoft requirements](https://learn.microsoft.com/en-us/windows/whats-new/windows-11-requirements)).
Canonical specifies 6 GB for Ubuntu Desktop 26.04
([release requirements](https://documentation.ubuntu.com/release-notes/26.04/#requirements-and-compatibility)).
Mint's own minimum is 2 GB, with 4 GB recommended; the 4 GB row above is our more
conservative product target, not Mint's minimum
([Mint FAQ](https://www.linuxmint.com/faq.php)).

Use a 64-bit processor meeting the chosen OS requirements; two CPU cores are a
practical minimum for responsive background work. Release builds use the portable
Rust target baseline, not `target-cpu=native`; AVX2 or a specific CPU vendor is not
an application requirement. An SSD is recommended for large trees and development,
but browsing supports HDDs and mounted network storage too.

Reserve **100 MiB** for the initial application payload and small settings, separate
from OS/runtime dependencies, plugins, thumbnails, copied files and build output.
This is a packaging allowance, not an installer-size measurement. A usable desktop
of **1280 x 800 logical pixels** is recommended; font size, scaling and touch density
affect how much fits. There is no independently qualified minimum display size yet.

Memory depends on directory entry counts, native icons, cached tabs, searches,
queued jobs and future plugins. Streaming copy/checksums must not load whole files
into RAM: copying a 100 GB file does not require 100 GB of memory. Bounded queues,
worker pools and cache budgets remain part of the design. The headroom above is
neither a process-memory cap nor a guarantee for arbitrarily large directories.

A short Windows 11 release startup smoke on 2026-11-06 observed **399 MiB peak
working set** and **611 MiB peak sampled private memory**, with Direct3D 12,
a large desktop window, the repository root and a three-entry Unicode fixture.
Private memory was sampled every 25 ms (69 samples); it is not the same metric as
resident physical RAM. This baseline predates final drag-and-drop integration and
is recorded in [verification](verification/2026-11-06-themes-and-commander-tools.md).
It rules out claiming 512 MiB as an established available-memory minimum, but does
not establish worst-case consumption or a Linux/macOS result.

## Runtime software

**All platforms:** a graphical desktop and readable system fonts covering the
scripts being displayed. **Hardware GPU acceleration is optional.** The preferred
renderer is Slint FemtoVG-WGPU: Direct3D 12 on Windows and normally Vulkan on Linux.
The CPU renderer remains available through `--software` and automatic startup
recovery when the default GPU cannot initialize. The recovery is limited to one
attempt before the first successful window reveal; it preserves original paths,
arguments and working directory. A failed hidden attempt cannot save preferences
or replacement geometry. No already-visible session or file operation is replayed.

Explicit `SLINT_BACKEND` choices are not silently overridden. `WGPU_BACKEND`
remains available for driver diagnosis. A graphical display is still necessary
with CPU rendering; this is not a headless application mode. GPU hashing is not
required. CPU scrolling uses the same pixel camera and bounded source loading,
with hardware-dependent rendering performance.

The pinned Slint software renderer omits bitmap emoji even when the font is
installed. That known glyph limitation and cross-platform font qualification are
separate from functional CPU startup; do not infer complete Unicode rendering
coverage from a smoke-test exit. For WGPU diagnostics, `SLINT_WGPU_CPU=1` allows a
CPU adapter while retaining the WGPU runtime. It is distinct from `--software`
and does not establish hardware-GPU performance. See the
[scrolling contract](developer/SCROLLING.md) and
[native evidence](NATIVE_SCROLL_VERIFICATION.md).

**Windows:** install the Microsoft Visual C++ x64 runtime if it is not already
present. The current MSVC release executable imports `VCRUNTIME140.dll` and the
Windows Universal CRT. Use Microsoft's
[supported Visual C++ redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist),
not individual DLL downloads. Rust, Cargo, Visual Studio, .NET and a separate Slint
SDK are **not** required to run a built executable. `cmd.exe` is supplied by Windows;
PowerShell must be installed to use that terminal choice.

**Linux:** use an X11 or Wayland desktop, fontconfig/FreeType, libxkbcommon,
the applicable X11/XCB or Wayland client libraries, libudev and graphics-driver
runtime libraries. Desktop integration also uses host tools such as `xdg-open`
and an installed terminal. Build prerequisites and package names are in the README.
External file drag-and-drop targets X11/XWayland; native Wayland needs separate
protocol integration, while in-application operations remain available.

The initial generic Linux build targets the **Ubuntu 24.04 / glibc 2.39** userspace baseline.
Build distributable Linux binaries on that baseline, and inspect their ELF versioned
dependencies before publication. Building on a newer distro can raise the required
glibc/libstdc++ versions; Rust's own older glibc minimum is not a guarantee for the
complete GUI application. Rebuilding from source may permit other configurations,
but does not make them qualified targets automatically. Fedora and openSUSE must
also pass native source builds and runtime checks; do not assume that an Ubuntu
binary is compatible merely because all three use glibc. Distribution RPM packages
are planned, not currently produced. Test Btrfs and ext4 plus the chosen mounted
network filesystems, keeping normal SELinux/AppArmor policy enabled. Do not create
separate Fedora/SUSE filesystem implementations: distro packaging sits above the
shared Linux platform adapter; capability detection handles filesystem differences.

Linux security-sensitive local operations use `openat2` (kernel **5.6+**) and
handle-relative APIs. Verified destination publication additionally requires
filesystem support for `O_TMPFILE` and `linkat`; no-overwrite rename requires the
corresponding native rename capability. A sufficiently new kernel alone does not
guarantee those features on every mounted filesystem. Unsupported operations fail
explicitly rather than silently weakening identity checks or checksum verification.
Newer optional metadata syscalls have guarded fallbacks; they do not raise the
general minimum kernel to the newest syscall's introduction version.

## Building and testing

Build-machine requirements are substantially higher than runtime requirements:

| Platform | Required development software | RAM / storage planning allowance |
| --- | --- | --- |
| Windows x64 | Pinned Rust **1.99.1**, MSVC C++ build tools, Windows SDK; PowerShell for repository scripts | **16 GB RAM recommended**; **32 GB** for concurrent builds or large test/link jobs. Reserve **20 GiB** free for one clean release build plus separate toolchain/download caches. |
| Linux x86-64 | Pinned Rust **1.99.1**, C/C++ linker toolchain, pkg-config and README development libraries | Same allowance. A POSIX local filesystem is needed for raw-filename and anonymous-file integrity tests; WSL's Windows-mounted paths do not exercise all Linux capabilities. |
| macOS | Not established; future Rust + Xcode Command Line Tools and native adapter | No verified build or RAM minimum yet. |

An **8 GB** development machine with `-j 1` and adequate pagefile/swap is a low-memory
configuration to try, **not a tested minimum**. Fat LTO can still need substantial
RAM for a single link; lowering Cargo parallelism cannot remove that cost. Normal
repository builds default to four jobs. See [release settings](RELEASE_BUILD.md).

Multiple OS/profile/test caches can consume **100 GiB or more**; the allowance for
one release build is not enough for an unlimited development history. Compiled,
generated and helper temporary output belongs under `_build/` (Linux shared-checkout
output under `_build/linux/`). Optional downloaded toolchains/dependency caches live
under `.tools/`; these are separate from disposable `_build/` output. Neither is
committed. Plugins must document any additional runtime/build requirements themselves.

Before promoting the RAM figures to measured minima, test the actual release binary
on memory-constrained machines: cold start, large Unicode listings, scrolling,
sorting, searches and concurrent verified copies. Record OS/driver, fixture sizes,
peak private memory/RSS, responsiveness and any swapping. A short startup smoke test
alone cannot establish those requirements.
