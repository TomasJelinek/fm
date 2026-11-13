#!/bin/sh
# Native host packages are changed only with explicit --install opt-in.
set -efu

usage() {
    printf '%s\n' 'Usage: sh scripts/setup-linux.sh [--install] [--dry-run] [--os-release FILE]'
    printf '%s\n' 'Supports Ubuntu/Mint, Fedora and openSUSE. Without --install, prints the package plan.'
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
    printf 'Unsupported Linux distribution: %s. Install the README prerequisites manually.\n' "$distribution_id" >&2
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
case "$package_manager" in
    apt-get)
        run_package_command apt-get update --error-on=any
        run_package_command apt-get install --no-install-recommends \
            build-essential python3 pkg-config curl ca-certificates gnupg xz-utils perl libclang-dev cmake ninja-build \
            libfontconfig1-dev libfreetype6-dev \
            libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libxcb1-dev libwayland-dev libudev-dev
        ;;
    dnf)
        run_package_command dnf install gcc gcc-c++ make python3 pkgconf-pkg-config \
            curl ca-certificates gnupg2 xz perl clang-libs clang-devel cmake ninja-build \
            fontconfig-devel freetype-devel libxkbcommon-devel libxkbcommon-x11-devel \
            libX11-devel libXcursor-devel libxcb-devel wayland-devel systemd-devel xdg-utils
        ;;
    zypper)
        run_package_command zypper install gcc gcc-c++ make python3 pkg-config xdg-utils \
            curl ca-certificates gpg2 xz perl clang-devel cmake ninja \
            'pkgconfig(fontconfig)' 'pkgconfig(freetype2)' 'pkgconfig(xkbcommon)' \
            'pkgconfig(xkbcommon-x11)' 'pkgconfig(x11)' 'pkgconfig(xcursor)' \
            'pkgconfig(xcb)' 'pkgconfig(wayland-client)' 'pkgconfig(libudev)'
        ;;
esac
