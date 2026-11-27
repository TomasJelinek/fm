#!/bin/sh
# Native host packages are changed only with explicit --install opt-in.
set -efu

usage() {
    printf '%s\n' 'Usage: sh scripts/setup-linux.sh [--install] [--dry-run] [--os-release FILE]'
    printf '%s\n' 'Supports Ubuntu/Mint, Fedora and openSUSE. Without --install, prints the package plan.'
    printf '%s\n' 'Installs the build packages, then the runtime libraries of the application and its HTML viewer.'
}

install=0
dry_run=0
os_release=/etc/os-release
while [ "$#" -gt 0 ]; do
    case "$1" in
        --install) install=1 ;;
        --dry-run) dry_run=1 ;;
        --os-release)
            if [ "$#" -lt 2 ]; then usage >&2; exit 2; fi
            shift
            os_release=$1
            ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
    shift
done
case "$(uname -s)" in
    Linux) ;;
    *) printf '%s\n' 'Native dependency setup requires Linux.' >&2; exit 2 ;;
esac
if [ ! -f "$os_release" ] || [ ! -r "$os_release" ]; then
    printf 'Cannot read Linux distribution metadata: %s\n' "$os_release" >&2
    exit 1
fi

# Treat os-release as data, never as executable shell input.
distribution_id=
distribution_like=
while IFS='=' read -r key value; do
    case "$key" in
        ID|ID_LIKE)
            value=${value#\"}
            value=${value%\"}
            value=${value#\'}
            value=${value%\'}
            case "$key" in
                ID) distribution_id=$value ;;
                ID_LIKE) distribution_like=$value ;;
            esac
            ;;
    esac
done < "$os_release"
package_manager=
for distribution in "$distribution_id" $distribution_like; do
    case "$distribution" in
        ubuntu|linuxmint|mint|debian) package_manager=apt-get; break ;;
        fedora) package_manager=dnf; break ;;
        opensuse|opensuse-leap|opensuse-tumbleweed|suse|sles|sled) package_manager=zypper; break ;;
    esac
done
if [ -z "$package_manager" ]; then
    printf 'Unsupported Linux distribution: %s. Install the prerequisites listed in docs/user-guide.md manually.\n' "$distribution_id" >&2
    exit 1
fi

run_package_command() {
    if [ "$(id -u)" != 0 ]; then set -- sudo -- "$@"; fi
    printf 'Command:'
    printf ' [%s]' "$@"
    printf '\n'
    if [ "$install" = 1 ] && [ "$dry_run" = 0 ]; then "$@"; fi
}

if [ "$install" = 0 ] || [ "$dry_run" = 1 ]; then
    printf '%s\n' 'Package plan only; use --install to install native dependencies.'
fi
# Runtime libraries, installed after the build packages: the application's desktop
# libraries, then those of the HTML viewer's Chromium (CEF), which the viewer names when
# one is missing (crates/midnight-worker-platform/src/browser_libraries_linux.rs; a test
# there checks that this plan installs every library it can name). RPM distributions
# install each by its soname, a capability of whichever package provides it. These words
# are split unquoted below, where `set -f` keeps them from being globbed.
rpm_runtime='libfontconfig.so.1()(64bit) libfreetype.so.6()(64bit) libxkbcommon.so.0()(64bit)
    libxkbcommon-x11.so.0()(64bit) libX11.so.6()(64bit) libXcursor.so.1()(64bit)
    libXi.so.6()(64bit) libXrandr.so.2()(64bit) libxcb.so.1()(64bit)
    libwayland-client.so.0()(64bit) libudev.so.1()(64bit) libvulkan.so.1()(64bit)
    libnss3.so()(64bit) libnssutil3.so()(64bit) libsmime3.so()(64bit) libnspr4.so()(64bit)
    libplc4.so()(64bit) libplds4.so()(64bit) libasound.so.2()(64bit) libatk-1.0.so.0()(64bit)
    libatk-bridge-2.0.so.0()(64bit) libatspi.so.0()(64bit) libcups.so.2()(64bit)
    libdbus-1.so.3()(64bit) libdrm.so.2()(64bit) libexpat.so.1()(64bit) libgbm.so.1()(64bit)
    libglib-2.0.so.0()(64bit) libgobject-2.0.so.0()(64bit) libgio-2.0.so.0()(64bit)
    libpango-1.0.so.0()(64bit) libcairo.so.2()(64bit) libXcomposite.so.1()(64bit)
    libXdamage.so.1()(64bit) libXext.so.6()(64bit) libXfixes.so.3()(64bit)'
case "$package_manager" in
    apt-get)
        run_package_command apt-get update --error-on=any
        run_package_command apt-get install --no-install-recommends \
            build-essential python3 pkg-config curl ca-certificates gnupg xz-utils perl libclang-dev cmake ninja-build \
            libfontconfig1-dev libfreetype6-dev \
            libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libxcb1-dev libwayland-dev libudev-dev
        # Package names of Ubuntu 24.04 / Mint 22 (the t64 names of the 64-bit time ABI).
        run_package_command apt-get install --no-install-recommends \
            libfontconfig1 libfreetype6 libxkbcommon0 libxkbcommon-x11-0 libx11-6 libxcursor1 \
            libxi6 libxrandr2 libxcb1 libwayland-client0 libudev1 libvulkan1 xdg-utils \
            libnss3 libnspr4 libasound2t64 libatk1.0-0t64 libatk-bridge2.0-0t64 libatspi2.0-0t64 \
            libcups2t64 libdbus-1-3 libdrm2 libexpat1 libgbm1 libglib2.0-0t64 libpango-1.0-0 \
            libcairo2 libxcomposite1 libxdamage1 libxext6 libxfixes3
        ;;
    dnf)
        run_package_command dnf install gcc gcc-c++ make python3 pkgconf-pkg-config \
            curl ca-certificates gnupg2 xz perl clang-libs clang-devel cmake ninja-build \
            fontconfig-devel freetype-devel libxkbcommon-devel libxkbcommon-x11-devel \
            libX11-devel libXcursor-devel libxcb-devel wayland-devel systemd-devel xdg-utils
        # shellcheck disable=SC2086 # one word per capability
        run_package_command dnf install $rpm_runtime
        ;;
    zypper)
        run_package_command zypper install gcc gcc-c++ make python3 pkg-config xdg-utils \
            curl ca-certificates gpg2 xz perl clang-devel cmake ninja \
            'pkgconfig(fontconfig)' 'pkgconfig(freetype2)' 'pkgconfig(xkbcommon)' \
            'pkgconfig(xkbcommon-x11)' 'pkgconfig(x11)' 'pkgconfig(xcursor)' \
            'pkgconfig(xcb)' 'pkgconfig(wayland-client)' 'pkgconfig(libudev)'
        # shellcheck disable=SC2086 # one word per capability
        run_package_command zypper install $rpm_runtime
        ;;
esac
