#!/usr/bin/env bash
# Allow the HTML viewer's Chromium sandbox on systems that restrict unprivileged user
# namespaces through AppArmor (Ubuntu 23.10 and later:
# kernel.apparmor_restrict_unprivileged_userns=1). Chromium's sandbox needs those
# namespaces; without this profile the HTML viewer cannot start. The profile has the
# form of Ubuntu's own browser profiles: it confines nothing and only grants `userns`
# to the viewer executable of one installation. Never disable the sandbox instead.
#
# Usage: sudo install-html-viewer-apparmor.sh [INSTALLATION]
#        sudo install-html-viewer-apparmor.sh --uninstall
#             install-html-viewer-apparmor.sh --print [INSTALLATION]
#
# INSTALLATION is a folder that contains viewers/midnight-viewer-html (an extracted
# package, /opt/midnight-explorer, ~/.local/bin or a build such as _build/debug), or a
# source checkout, whose builds under _build/*/ are all covered. Without it, the
# installation is the folder of this script, or the checkout whose scripts/ holds it.
# The machine has one such profile, so the sandbox is allowed for one installation at a
# time: installing for another installation replaces it (the script says which one
# loses it), and installing again for the same one changes nothing.
set -euo pipefail

PROFILE=midnight-explorer-html-viewer
POLICY=/etc/apparmor.d
TARGET="$POLICY/$PROFILE"
VIEWER=viewers/midnight-viewer-html/midnight-viewer-html

fail() {
    echo "install-html-viewer-apparmor: $*" >&2
    exit 1
}

usage() {
    cat >&2 <<'EOF'
Usage: sudo install-html-viewer-apparmor.sh [INSTALLATION]
       sudo install-html-viewer-apparmor.sh --uninstall
            install-html-viewer-apparmor.sh --print [INSTALLATION]
EOF
    exit 2
}

mode=install
installation=
for argument in "$@"; do
    case "$argument" in
        --uninstall) mode=uninstall ;;
        --print) mode=print ;;
        -h | --help) usage ;;
        -*) usage ;;
        *) [ -z "$installation" ] || usage; installation=$argument ;;
    esac
done
[ "$mode" != uninstall ] || [ -z "$installation" ] || usage

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
script_path=$(readlink -f -- "${BASH_SOURCE[0]}")

# Whether group $2 is the private group of user $1. Distributions with user private
# groups (Debian, Ubuntu, Fedora) give every user a group of their own and, through
# pam_umask or the shell profile, a umask of 002, so the folders a user creates are
# writable by that group and yet by nobody else: the user's primary group, named like the
# user, which is no other user's primary group and lists no other member. Only the local
# files tell ($3 and $4, normally /etc/passwd and /etc/group), as for pam_umask; a user
# only a directory service knows, or files that include one (lines starting with + or -),
# have none. The viewer applies the same rule.
private_group() {
    local passwd=${3:-/etc/passwd}
    LC_ALL=C awk -F: -v uid="$1" -v gid="$2" -v passwd="$passwd" '
        /^[+-]/ { included = 1 }
        FILENAME == passwd {
            if ($3 ~ /^[0-9]+$/ && $4 ~ /^[0-9]+$/) {
                users++
                name[users] = $1
                id[users] = $3 + 0
                primary[users] = $4 + 0
            }
            next
        }
        !found && $3 ~ /^[0-9]+$/ && $3 + 0 == gid + 0 {
            found = 1
            group = $1
            members = $4
        }
        END {
            if (included || !found) exit 1
            for (i = 1; i <= users; i++)
                if (id[i] == uid + 0) { user = i; break }
            if (!user || primary[user] != gid + 0 || group != name[user]) exit 1
            for (i = 1; i <= users; i++)
                if (primary[i] == gid + 0 && name[i] != name[user]) exit 1
            count = split(members, member, ",")
            for (i = 1; i <= count; i++)
                if (member[i] != "" && member[i] != name[user]) exit 1
            exit 0
        }' "$passwd" "${4:-/etc/group}"
}

# The first of folder $1 and the folders above it that users other than its owner can
# write to (even a sticky one such as /tmp; its group only if that is not the owner's
# private group), or that cannot be inspected; nothing (and status 1) when there is none.
shared_folder() {
    local folder=$1 status mode owner group
    while :; do
        if ! status=$(stat -c '%a %u %g' -- "$folder") ||
            ! read -r mode owner group <<<"$status" ||
            [ $((8#$mode & 8#002)) -ne 0 ] ||
            { [ $((8#$mode & 8#020)) -ne 0 ] && ! private_group "$owner" "$group"; }; then
            printf '%s\n' "$folder"
            return 0
        fi
        [ "$folder" != / ] || return 1
        folder=$(dirname -- "$folder")
    done
}

# Whether only root can change $1 and every folder above it: no link, owned by root and
# neither group- nor other-writable. Running a script that someone else could have replaced
# as root would let them run anything as root.
only_root_can_change() {
    local path=$1 owner mode
    while :; do
        [ ! -L "$path" ] || return 1
        owner=$(stat -c %u -- "$path") && mode=$(stat -c %a -- "$path") || return 1
        [ "$owner" = 0 ] && [ $((8#$mode & 8#022)) -eq 0 ] || return 1
        [ "$path" != / ] || return 0
        path=$(dirname -- "$path")
    done
}

# The executable the profile attaches to: one exact path, or every build of a checkout.
attachment() {
    local root=$1 path folder shared
    if [ -z "$root" ]; then
        if [ -f "$script_dir/$VIEWER" ]; then
            root=$script_dir
        elif [ "$(basename -- "$script_dir")" = scripts ] && [ -d "$script_dir/../_build" ]; then
            root=$script_dir/..
        else
            fail "no installation beside this script; pass the folder that contains viewers/"
        fi
    fi
    [ -d "$root" ] || fail "$root is not a folder"
    root=$(cd -- "$root" && pwd -P)
    case "$root" in
        *[!A-Za-z0-9._+/@:\ -]*)
            fail "$root contains characters that AppArmor reads as patterns; use a path of letters, digits, spaces and ._+-@: instead" ;;
    esac
    if [ -f "$root/$VIEWER" ]; then
        path=$root/$VIEWER folder=$root/${VIEWER%/*}
    elif [ -f "$root/midnight-viewer-html" ] && [ "$(basename -- "$root")" = midnight-viewer-html ]; then
        path=$root/midnight-viewer-html folder=$root
    elif [ -d "$root/_build" ] && [ -f "$root/Cargo.toml" ]; then
        path=$root/_build/*/$VIEWER folder=$root/_build
    else
        fail "$root contains neither $VIEWER nor a source checkout's _build folder"
    fi
    if shared=$(shared_folder "$folder"); then
        fail "users other than its owner can write to $shared, so they could put another program at the viewer's path, now or once the installation is gone; use an installation in folders that only their owners can change"
    fi
    printf '%s\n' "$path"
}

profile_text() {
    local path=$1
    cat <<EOF
# Midnight Explorer HTML viewer: lets its Chromium (CEF) runtime create the user
# namespaces of Chromium's sandbox where AppArmor restricts them for unconfined
# programs. Like Ubuntu's profiles for other browsers, it confines nothing else.
# Written by install-html-viewer-apparmor.sh; remove with its --uninstall option.

abi <abi/4.0>,
include <tunables/global>

profile $PROFILE "$path" flags=(unconfined) {
  userns,

  # Site-specific additions and overrides. See local/README for details.
  include if exists <local/$PROFILE>
}
EOF
}

# Other files that define this profile and nothing else, such as one written by hand,
# each name ended by a NUL byte (names may contain any other character).
earlier_definitions() {
    local file
    for file in "$POLICY"/*; do
        [ -f "$file" ] && [ "$file" != "$TARGET" ] || continue
        grep -Eq -e "^[[:space:]]*profile[[:space:]]+$PROFILE[[:space:]]" -- "$file" || continue
        if [ "$(grep -Ec -e '^[[:space:]]*profile[[:space:]]' -- "$file")" = 1 ]; then
            printf '%s\0' "$file"
        else
            echo "Note: $file also defines other profiles; remove its $PROFILE profile by hand." >&2
        fi
    done
}

# The executable path the installed profile attaches to (quoted or not), if any.
installed_attachment() {
    [ -f "$TARGET" ] || return 0
    sed -n -e 's/^profile '"$PROFILE"' "\{0,1\}\([^"]*\)"\{0,1\} flags=.*/\1/p' -- "$TARGET"
}

loaded() {
    [ -r /sys/kernel/security/apparmor/profiles ] &&
        grep -q "^$PROFILE (" /sys/kernel/security/apparmor/profiles
}

require_root() {
    [ "$(id -u)" != 0 ] || return 0
    if only_root_can_change "$script_path"; then
        fail "this changes the system's AppArmor policy and needs root. Run: sudo$(printf ' %q' "$script_path" "$@")"
    fi
    fail "this changes the system's AppArmor policy and needs root, but do not run this copy with sudo: a program running as you could have replaced it. Use Install... in the HTML viewer, or install Midnight Explorer system-wide and run that installation's copy as root."
}

parser() {
    command -v apparmor_parser || fail "apparmor_parser is missing (package apparmor)"
}

if [ "$mode" = uninstall ]; then
    require_root --uninstall
    apparmor_parser=$(parser)
    removed=0
    remove_definition() {
        [ -f "$1" ] || return 0
        "$apparmor_parser" -R "$1" 2>/dev/null || true
        rm -f -- "$1"
        echo "Removed $1"
        removed=1
    }
    remove_definition "$TARGET"
    while IFS= read -r -d '' file; do
        remove_definition "$file"
    done < <(earlier_definitions)
    if loaded; then
        printf '%s' "$PROFILE" > /sys/kernel/security/apparmor/.remove
        echo "Unloaded the $PROFILE profile"
        removed=1
    fi
    [ "$removed" = 1 ] || echo "The $PROFILE profile was not installed."
    exit 0
fi

path=$(attachment "$installation")

if [ "$mode" = print ]; then
    profile_text "$path"
    exit 0
fi

require_root ${installation:+"$installation"}
[ "$(cat /sys/module/apparmor/parameters/enabled 2>/dev/null)" = Y ] ||
    fail "AppArmor is not enabled, so it does not restrict the sandbox; no profile is needed"
[ -f "$POLICY/abi/4.0" ] ||
    fail "this AppArmor has no userns rule (AppArmor 4, Ubuntu 23.10 or later); no profile is needed"
apparmor_parser=$(parser)
restriction=$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns 2>/dev/null || echo 0)
[ "$restriction" = 1 ] ||
    echo "Note: this system does not restrict user namespaces now; the profile takes effect if it does."

case "$path" in
    *'*'*) directory=${path%%/_build/*}/_build ;;
    *) directory=$(dirname -- "$path") ;;
esac
if ! only_root_can_change "$directory"; then
    echo "Note: $directory or a folder above it can be changed without root, so any program" \
         "placed at the viewer's path there may create user namespaces. Prefer a root-owned" \
         "installation, such as /opt/midnight-explorer, on shared computers."
fi

previous=$(installed_attachment || true)
previous=${previous%%$'\n'*}
staged=$(mktemp)
trap 'rm -f -- "$staged"' EXIT
profile_text "$path" > "$staged"
"$apparmor_parser" -QTK "$staged" >/dev/null || fail "AppArmor refused the generated profile"
install -m 0644 -o root -g root -- "$staged" "$TARGET"
while IFS= read -r -d '' file; do
    rm -f -- "$file"
    echo "Removed the earlier definition in $file"
done < <(earlier_definitions)
"$apparmor_parser" -r "$TARGET"
echo "Installed $TARGET for $path."
if [ -n "$previous" ] && [ "$previous" != "$path" ]; then
    echo "It replaced the profile for $previous: one installation at a time can use the" \
         "HTML viewer here, and that one shows the missing permission again."
fi
echo "Open the HTML file again; restarting Midnight Explorer is not needed."
