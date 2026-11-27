#!/usr/bin/env python3
"""Install, upgrade, repair and remove the Windows MSI for real, on a disposable machine.

`scripts/package-msi.py` checks the MSI table by table and file by file and rehearses the
actions before its first dialog, but nothing there installs it. This script does, with
`msiexec`, in both scopes (Just for me and For all users), and checks what Windows
Installer left behind: every file byte for byte against the release ZIP, the
`Software\\Midnight Explorer Installer` key, the Start menu shortcut, the Add/Remove
Programs entry, the folder permissions of an all-users copy, an upgrade from an older
build and from the published per-user MSIs, a refused downgrade, a repair, both scopes
side by side and a complete removal that keeps the settings folder. The installed
executable runs its own headless and UI smoke tests.

It changes the machine it runs on (installs, uninstalls, writes HKCU and HKLM) and so
refuses to run without --disposable-machine. CI runs it on a throwaway GitHub-hosted
Windows runner (an administrator). Never run it on a machine whose Midnight Explorer
installation or settings matter; it also refuses to start when any copy of the
product is installed. `run --dry-run` prints the plan without changing anything.

Commands:
  stand-in   a release-shaped ZIP of a release build (the CI release job's executable,
             helper, viewer and filesystem packages), for jobs that do not package releases
  prepare    the MSI of a release ZIP and of an older-versioned copy of it
  previous   download the published per-user MSIs older than the workspace version and
             check them against their signed SHA256SUMS
  ice        Windows SDK ICE validation (darice.cub), when the SDK is installed
  run        the installation tests (or the plan, with --dry-run)
"""

import argparse
import ctypes
import hashlib
import importlib.util
import json
import os
import re
import secrets
import shutil
import struct
import subprocess
import sys
import zipfile
from pathlib import Path

if os.name == "nt":  # the Windows Installer API; the pure logic is tested on Linux too
    from ctypes import wintypes

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))


def _load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


package_msi = _load("package_msi", "package-msi.py")
package_release = package_msi.package_release

PRODUCT = package_msi.PRODUCT
PUBLISHER = package_msi.PUBLISHER
EXECUTABLE = package_msi.EXECUTABLE
INSTALL_FOLDER = package_msi.INSTALL_FOLDER
REGISTRY_KEY = package_msi.REGISTRY_KEY
UPGRADE_CODE = package_msi.UPGRADE_CODE
TARGET = package_msi.TARGET
# The older build that the upgrade and downgrade tests install: the same payload under a
# version that no release uses.
OLDER_VERSION = "0.0.1"
# Published per-user MSIs that a newer MSI must upgrade in place (before the dual-purpose
# package; each is checked against its release's signed SHA256SUMS).
PUBLISHED_MSIS = ("0.6.1", "0.6.2")
SHORTCUT = f"{PRODUCT}.lnk"
SETTINGS_FOLDER = "Midnight Explorer"
MARKER = "msi-install-test-marker.txt"
DOWNGRADE_TEXT = f"A newer version of {PRODUCT} is already installed"
UNINSTALL_KEY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"
SCOPES = ("user", "machine")
# Exit codes of msiexec that mean success (3010: a restart completes it).
SUCCESS = (0, 3010)
ERROR_INSTALL_FAILURE = 1603
MSIEXEC_TIMEOUT = 900
SMOKE_TIMEOUT = 180
STAND_IN_NOTICE = ("Stand-in THIRD-PARTY.txt of the MSI installation test "
                   "(scripts/test-msi-install.py); not a release.\n")
SCENARIOS = ("scopes", "previous", "ui")

# Access rights that let a principal change a file or folder (FILE_WRITE_DATA/ADD_FILE,
# FILE_APPEND_DATA/ADD_SUBDIRECTORY, FILE_WRITE_EA, FILE_DELETE_CHILD,
# FILE_WRITE_ATTRIBUTES, DELETE, WRITE_DAC, WRITE_OWNER, GENERIC_ALL, GENERIC_WRITE).
WRITE_RIGHTS = 0x2 | 0x4 | 0x10 | 0x40 | 0x100 | 0x10000 | 0x40000 | 0x80000 | 0x10000000 | 0x40000000
# Principals that may change Program Files: SYSTEM, Administrators, TrustedInstaller,
# and CREATOR OWNER (applies only to what such a principal creates).
ADMINISTRATIVE_SIDS = frozenset({
    "S-1-5-18", "S-1-5-32-544", "S-1-3-0",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
})

# ICE findings of the current package that are known and recorded (REL-05); any other
# ICE error or failure fails the check. (ICE, text the message contains.)
KNOWN_ICE_FINDINGS = (
    ("ICE03", "Missing specifications in _Validation Table"),
    ("ICE20", "Standard Dialog: 'FilesInUse' not found in Dialog table"),
    ("ICE20", "ErrorDialog Property not specified in Property table"),
    ("ICE20", "dialog/action not found in 'AdminUISequence' Sequence Table"),
    ("ICE57", "Component 'StartMenuShortcut' has both per-user data and a keypath"),
)
ICE_TYPES = {"0": "failure", "1": "error", "2": "warning", "3": "info"}


class Failure(Exception):
    """A check that failed; the message says what was found."""


# Pure logic ------------------------------------------------------------------------------


def refuse_unless_disposable(disposable, dry_run):
    if dry_run:
        return
    if not disposable:
        raise Failure("Refused: this test installs and removes Midnight Explorer and changes HKCU and "
                      "HKLM. Run it only on a throwaway machine (CI's hosted runner), with "
                      "--disposable-machine.")


def parse_version(text):
    parts = text.split(".")
    if len(parts) != 3 or not all(part.isdigit() for part in parts):
        raise ValueError(f"Not a version: {text}")
    return tuple(int(part) for part in parts)


def previous_versions(current, published=PUBLISHED_MSIS):
    """The published per-user MSIs older than `current`, oldest first."""
    return [version for version in sorted(published, key=parse_version)
            if parse_version(version) < parse_version(current)]


def scope_properties(scope):
    """The command-line properties that choose the scope of the dual-purpose package."""
    if scope == "user":
        return ["ALLUSERS=2", "MSIINSTALLPERUSER=1"]
    if scope == "machine":
        return ["ALLUSERS=1"]
    if scope is None:
        return []
    raise ValueError(f"Unknown scope: {scope}")


def msiexec_arguments(operation, package, log, scope=None):
    """msiexec's arguments (without the executable) for a silent operation with a verbose log."""
    options = {"install": ["/i"], "repair": ["/fomus"], "uninstall": ["/x"]}
    if operation not in options:
        raise ValueError(f"Unknown operation: {operation}")
    if operation != "install" and scope is not None:
        raise ValueError("Only an installation chooses its scope")
    return [*options[operation], str(package), "/qn", "/norestart", "/l*v", str(log),
            *scope_properties(scope)]


def decode_log(data):
    """An msiexec log as text: UTF-16 with a byte order mark, otherwise the ANSI code page."""
    if data.startswith(b"\xff\xfe"):
        return data[2:].decode("utf-16-le", errors="replace")
    if data.startswith(b"\xef\xbb\xbf"):
        return data[3:].decode("utf-8", errors="replace")
    return data.decode("cp1252", errors="replace")


def downgrade_refused(code, log_text):
    return code == ERROR_INSTALL_FAILURE and DOWNGRADE_TEXT in log_text


def install_folder(scope, environment):
    if scope == "user":
        return Path(environment["LOCALAPPDATA"]) / "Programs" / INSTALL_FOLDER
    return Path(environment["ProgramW6432"]) / INSTALL_FOLDER


def shortcut_path(scope, environment):
    base = environment["APPDATA"] if scope == "user" else environment["ProgramData"]
    return Path(base) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / SHORTCUT


def expected_working_folder(scope, environment):
    """Per user the installing user's profile; per machine the literal variable."""
    return environment["USERPROFILE"] if scope == "user" else "%USERPROFILE%"


def same_path(first, second):
    def normal(path):
        return os.path.normcase(str(path)).rstrip("\\/").casefold()
    return normal(first) == normal(second)


def compare_files(expected, actual):
    """Differences between two {relative path: SHA-256} maps (missing, extra, changed)."""
    problems = [f"missing {name}" for name in sorted(expected.keys() - actual.keys())]
    problems += [f"not in the release payload: {name}" for name in sorted(actual.keys() - expected.keys())]
    problems += [f"differs from the release payload: {name}" for name in sorted(expected.keys() & actual.keys())
                 if expected[name] != actual[name]]
    return problems


def check_uninstall_entry(values, version, folder, icon_exists, registry=True):
    """Problems of an Installed apps (Add/Remove Programs) entry, by registry value name."""
    problems = []
    expected = [("DisplayName", PRODUCT), ("DisplayVersion", version), ("Publisher", PUBLISHER)]
    if registry:
        expected += [("WindowsInstaller", 1), ("NoModify", 1)]
    for name, wanted in expected:
        if values.get(name) != wanted:
            problems.append(f"{name} is {values.get(name)!r}, expected {wanted!r}")
    if not same_path(values.get("InstallLocation", ""), folder):
        problems.append(f"InstallLocation is {values.get('InstallLocation')!r}, expected {str(folder)!r}")
    icon = str(values.get("DisplayIcon", ""))
    if not icon.lower().endswith(".ico") or not icon_exists(icon):
        problems.append(f"DisplayIcon {icon!r} is not an existing icon file")
    return problems


def check_shortcut(info, target, working_folder):
    problems = []
    if not same_path(info.get("target") or "", target):
        problems.append(f"target is {info.get('target')!r}, expected {str(target)!r}")
    if not same_path(info.get("working_folder") or "", working_folder):
        problems.append(f"start folder is {info.get('working_folder')!r}, expected {working_folder!r}")
    if info.get("arguments"):
        problems.append(f"unexpected arguments {info['arguments']!r}")
    return problems


def parse_shortcut(data):
    """Target, start folder and arguments of a shell link (.lnk, [MS-SHLLINK])."""
    if len(data) < 76 or struct.unpack_from("<I", data, 0)[0] != 0x4C:
        raise ValueError("Not a shell link")
    flags = struct.unpack_from("<I", data, 20)[0]
    unicode = bool(flags & 0x80)
    offset = 76
    if flags & 0x1:  # HasLinkTargetIDList
        offset += 2 + struct.unpack_from("<H", data, offset)[0]
    target = None
    if flags & 0x2:  # HasLinkInfo
        size, header, info_flags, _volume, base, _network, suffix = struct.unpack_from("<7I", data, offset)
        if info_flags & 0x1:
            if header >= 0x24:
                base_unicode, suffix_unicode = struct.unpack_from("<2I", data, offset + 28)
                target = _wide(data, offset + base_unicode) + _wide(data, offset + suffix_unicode)
            else:
                target = _narrow(data, offset + base) + _narrow(data, offset + suffix)
        offset += size
    strings = {}
    for flag, name in ((0x4, "name"), (0x8, "relative_path"), (0x10, "working_folder"),
                       (0x20, "arguments"), (0x40, "icon")):
        if flags & flag:
            count = struct.unpack_from("<H", data, offset)[0]
            offset += 2
            width = 2 if unicode else 1
            raw = data[offset:offset + count * width]
            strings[name] = raw.decode("utf-16-le" if unicode else "cp1252")
            offset += count * width
    # ExtraData: an EnvironmentVariableDataBlock holds a target with variables.
    while offset + 8 <= len(data):
        size, signature = struct.unpack_from("<2I", data, offset)
        if size < 8:
            break
        if signature == 0xA0000001 and target is None and size >= 0x314:
            target = data[offset + 268:offset + 788].decode("utf-16-le").split("\0", 1)[0] or None
        offset += size
    return {"target": target, "working_folder": strings.get("working_folder"),
            "arguments": strings.get("arguments")}


def _wide(data, offset):
    end = offset
    while data[end:end + 2] != b"\0\0":
        end += 2
    return data[offset:end].decode("utf-16-le")


def _narrow(data, offset):
    return data[offset:data.index(b"\0", offset)].decode("cp1252")


def write_grants(objects, baseline):
    """Entries that let a non-administrative principal change an installed file or folder
    and that Program Files itself (`baseline`) does not have.

    Each object is {"Path", "Rules": [{"Sid", "Rights", "Allow", "InheritOnly"}]}.
    """
    allowed = {(rule["Sid"], int(rule["Rights"]) & 0xFFFFFFFF) for item in baseline for rule in item["Rules"]
               if rule["Allow"]}
    problems = []
    for item in objects:
        for rule in item["Rules"]:
            rights = int(rule["Rights"]) & 0xFFFFFFFF  # PowerShell writes it as a signed value
            if (rule["Allow"] and rights & WRITE_RIGHTS and rule["Sid"] not in ADMINISTRATIVE_SIDS
                    and (rule["Sid"], rights) not in allowed):
                problems.append(f"{item['Path']}: {rule['Sid']} may change it (rights 0x{rights:08X})")
    return problems


def parse_ice_message(text):
    """(ICE, type, description) of an ICE's user message, or None for other messages."""
    fields = text.split("\t")
    if len(fields) < 3 or not re.fullmatch(r"ICE\d+", fields[0]) or fields[1] not in ICE_TYPES:
        return None
    return fields[0], ICE_TYPES[fields[1]], fields[2]


def ice_findings(messages, known=KNOWN_ICE_FINDINGS):
    """(unexpected errors and failures, known findings, warnings) of the ICE messages."""
    unexpected, recorded, warnings = [], [], []
    for message in messages:
        parsed = parse_ice_message(message)
        if parsed is None:
            continue
        ice, kind, description = parsed
        if kind in ("error", "failure"):
            if any(ice == known_ice and text in description for known_ice, text in known):
                recorded.append(parsed)
            else:
                unexpected.append(parsed)
        elif kind == "warning":
            warnings.append(parsed)
    return unexpected, recorded, warnings


def known_counts(recorded, known=KNOWN_ICE_FINDINGS):
    """{(ICE, known text): number of messages} of the recorded findings."""
    counts = {}
    for ice, _kind, description in recorded:
        entry = next((ice, text) for known_ice, text in known if ice == known_ice and text in description)
        counts[entry] = counts.get(entry, 0) + 1
    return counts


def rewrite_version(text, old, new):
    """README text of `old` rewritten for `new` (its first line and source link)."""
    return text.replace(f"{PRODUCT} {old} ", f"{PRODUCT} {new} ").replace(f"/tree/v{old}", f"/tree/v{new}")


def plan(scenarios, version, previous, ui=False, ui_smoke=False):
    """The steps of a run, in order, as (operation, details) tuples.

    The packages are "new" (the release MSI), "older" (the same payload as OLDER_VERSION)
    and "previous-<version>" (a published per-user MSI).
    """
    steps = [("preflight", {}), ("marker", {})]
    if "scopes" in scenarios:
        for scope in SCOPES:
            steps += [
                ("install", {"package": "older", "scope": scope}),
                ("check-installed", {"package": "older", "scope": scope}),
                ("install", {"package": "new", "scope": scope}),
                ("check-replaced", {"package": "older", "scope": scope}),
                ("check-installed", {"package": "new", "scope": scope}),
                ("install-refused", {"package": "older", "scope": scope}),
                ("check-installed", {"package": "new", "scope": scope}),
                ("smoke", {"scope": scope, "ui": ui_smoke}),
                ("repair", {"package": "new", "scope": scope}),
                ("check-installed", {"package": "new", "scope": scope}),
            ]
            if scope == "machine":
                steps.append(("check-permissions", {"scope": scope}))
            else:
                # One scope at a time, so that repair and removal cannot pick the other.
                steps += [("uninstall", {"package": "new", "scope": scope}),
                          ("check-removed", {"scope": scope})]
        # Just for me beside the all-users copy: both stay, each is removed on its own.
        steps += [("install", {"package": "new", "scope": "user"}),
                  ("check-side-by-side", {}),
                  ("uninstall-one", {"package": "new"}),
                  ("uninstall-one", {"package": "new"}),
                  ("check-marker", {})]
    if "previous" in scenarios:
        for published in previous:
            steps += [
                ("install", {"package": f"previous-{published}", "scope": None}),
                ("check-published", {"package": f"previous-{published}"}),
                ("install", {"package": "new", "scope": "user"}),
                ("check-replaced", {"package": f"previous-{published}", "scope": "user"}),
                ("check-installed", {"package": "new", "scope": "user"}),
                ("uninstall", {"package": "new", "scope": "user"}),
                ("check-removed", {"scope": "user"}),
            ]
        steps.append(("check-marker", {}))
    if "ui" in scenarios or ui:
        for scope in SCOPES:
            steps += [
                ("ui-install", {"package": "new", "scope": scope}),
                ("check-installed", {"package": "new", "scope": scope}),
                ("uninstall", {"package": "new", "scope": scope}),
                ("check-removed", {"scope": scope}),
            ]
        steps.append(("check-marker", {}))
    steps.append(("check-nothing-installed", {}))
    return steps


# Release-shaped payloads ------------------------------------------------------------------


def sha256_file(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def payload_hashes(archive):
    """{relative path: SHA-256} of what the MSI of `archive` installs."""
    payload = package_msi.read_payload(archive)
    hashes = {}
    with zipfile.ZipFile(archive) as bundle:
        for relative, _info in payload.files:
            digest = hashlib.sha256()
            with bundle.open(f"{payload.root}/{relative}") as source:
                for chunk in iter(lambda: source.read(1 << 20), b""):
                    digest.update(chunk)
            hashes[relative] = digest.hexdigest()
    return payload, hashes


def _zip_entry(bundle, name, data):
    info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
    info.create_system = 3
    info.external_attr = 0o100644 << 16
    info.compress_type = zipfile.ZIP_DEFLATED
    bundle.writestr(info, data)


def stand_in(executable, helper, viewers, filesystems, output, repository=ROOT, out=sys.stdout):
    """A ZIP with the release layout and checks of package-release.py, made from a release
    build and this run's plugin packages (no thumbnailers, stand-in notices)."""
    version = package_msi.workspace_version(repository)
    output = package_release.build_output(output, repository)
    root = f"midnight-explorer-{version}-{TARGET}"
    archive = output / f"{root}.zip"
    if archive.exists():
        raise Failure(f"Refused: {archive} exists")
    readme = (repository / "packaging/windows/README.txt").read_text(encoding="utf-8")
    source = f"{package_msi.SOURCE_REPOSITORY}/tree/v{version}"
    if source not in readme:
        readme += f"\nSource: {source}\n"
    files = {
        EXECUTABLE: Path(executable).read_bytes(),
        "midnight-elevated-listing.exe": Path(helper).read_bytes(),
        "README.txt": readme.encode("utf-8"),
        "LICENSE.txt": (repository / "LICENSE").read_bytes(),
        "THIRD-PARTY.txt": STAND_IN_NOTICE.encode("ascii"),
    }
    for folder, prefix in ((viewers, "viewers"), (filesystems, "filesystems")):
        folder = Path(folder)
        for path in sorted(folder.rglob("*")):
            if path.is_symlink() or getattr(path, "is_junction", lambda: False)():
                raise Failure(f"Refused: a link in a plugin package: {path}")
            if path.is_file():
                files[f"{prefix}/{path.relative_to(folder).as_posix()}"] = path.read_bytes()
    files["BUILD-INFO.json"] = json.dumps({
        "version": version, "target": TARGET, "binary": EXECUTABLE,
        "binary_sha256": hashlib.sha256(files[EXECUTABLE]).hexdigest(),
        "stand_in": "scripts/test-msi-install.py",
    }, indent=2).encode("utf-8")
    output.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive, "x") as bundle:
        for name in sorted(files):
            _zip_entry(bundle, f"{root}/{name}", files[name])
    package_msi.read_payload(archive, expected_version=version)
    print(archive, file=out)
    return archive


def older_copy(archive, version, folder):
    """The release ZIP as `version`: its root, BUILD-INFO.json and README.txt rewritten."""
    payload = package_msi.read_payload(archive)
    root = f"midnight-explorer-{version}-{TARGET}"
    target = Path(folder) / f"{root}.zip"
    with zipfile.ZipFile(archive) as source, zipfile.ZipFile(target, "x") as bundle:
        for info in source.infolist():
            relative = info.filename.split("/", 1)[1]
            data = source.read(info)
            if relative == "BUILD-INFO.json":
                build_info = json.loads(data)
                build_info["version"] = version
                data = json.dumps(build_info, indent=2).encode("utf-8")
            elif relative == "README.txt":
                data = rewrite_version(data.decode("utf-8"), payload.version, version).encode("utf-8")
            _zip_entry(bundle, f"{root}/{relative}", data)
    return target


def prepare(archive, work, out=sys.stdout):
    """The MSI of the release ZIP and of an OLDER_VERSION copy, and a manifest of both."""
    archive = Path(archive).absolute()
    work = package_release.build_output(work, ROOT)
    if work.exists():
        raise Failure(f"Refused: {work} exists")
    payload = package_msi.read_payload(archive, expected_version=package_msi.workspace_version())
    if parse_version(OLDER_VERSION) >= parse_version(payload.version):
        raise Failure(f"The older build {OLDER_VERSION} must be older than {payload.version}")
    work.mkdir(parents=True)
    new = package_msi.build(archive, work / "new", out=out)
    # package-msi.py builds the workspace version below a checkout's _build: the older
    # copy gets a checkout of its own that names OLDER_VERSION.
    repository = work / "older"
    (repository / "_build").mkdir(parents=True)
    (repository / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{OLDER_VERSION}"\n',
                                           encoding="utf-8")
    older_archive = older_copy(archive, OLDER_VERSION, repository / "_build")
    older = package_msi.build(older_archive, repository / "_build" / "msi", repository=repository, out=out)
    manifest = {"version": payload.version, "new": {"msi": str(new), "zip": str(archive)},
                "older": {"msi": str(older), "zip": str(older_archive), "version": OLDER_VERSION}}
    (work / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


# Windows: msiexec, registry, files ---------------------------------------------------------

INSTALLCONTEXT = {"user": 2, "machine": 4}  # MSIINSTALLCONTEXT_USERUNMANAGED, _MACHINE
ERROR_UNKNOWN_PRODUCT = 1605
ERROR_UNKNOWN_PROPERTY = 1608
ERROR_MORE_DATA = 234


def product_info(product_code, scope, name):
    """A property of the product installed in that scope's context (None when it is not)."""
    import msi_database
    msi = msi_database._library()
    function = msi.MsiGetProductInfoExW
    function.argtypes = (wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD, wintypes.LPCWSTR,
                         wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD))
    function.restype = wintypes.UINT
    buffer = ctypes.create_unicode_buffer(1024)
    size = wintypes.DWORD(len(buffer))
    result = function(product_code, None, INSTALLCONTEXT[scope], name, buffer, ctypes.byref(size))
    if result in (ERROR_UNKNOWN_PRODUCT, ERROR_UNKNOWN_PROPERTY):
        return None
    if result != 0:
        raise Failure(f"MsiGetProductInfoEx({product_code}, {scope}, {name}) failed with {result}")
    return buffer.value


def product_version(product_code, scope):
    return product_info(product_code, scope, "VersionString")


def entry_values(product_code, scope):
    """What Installed apps shows for the product in that scope: its Uninstall registry key
    when Windows Installer wrote one there, otherwise Windows Installer's own record."""
    values = registry_values(scope, f"{UNINSTALL_KEY}\\{product_code}")
    if values is not None:
        return "registry", values
    if product_version(product_code, scope) is None:
        return None, None
    return "msi", {name: product_info(product_code, scope, source) for name, source in (
        ("DisplayName", "InstalledProductName"), ("DisplayVersion", "VersionString"),
        ("Publisher", "Publisher"), ("InstallLocation", "InstallLocation"), ("DisplayIcon", "ProductIcon"))}


def related_products():
    """Product codes of every installed product with the package's UpgradeCode."""
    import msi_database
    msi = msi_database._library()
    function = msi.MsiEnumRelatedProductsW
    function.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPWSTR)
    function.restype = wintypes.UINT
    found, index = [], 0
    while True:
        buffer = ctypes.create_unicode_buffer(39)
        result = function(UPGRADE_CODE, 0, index, buffer)
        if result == 259:  # ERROR_NO_MORE_ITEMS
            return found
        if result != 0:
            raise Failure(f"MsiEnumRelatedProducts failed with {result}")
        found.append(buffer.value)
        index += 1


def package_property(msi_path, name):
    import msi_database
    with msi_database.Database(msi_path) as database:
        rows = database.rows(f"SELECT `Value` FROM `Property` WHERE `Property` = '{name}'")
    if len(rows) != 1:
        raise Failure(f"{msi_path} has no {name}")
    return rows[0][0]


def registry_values(scope, key):
    """The values of a key (64-bit view) in HKCU (user) or HKLM (machine), or None."""
    import winreg
    hive = winreg.HKEY_CURRENT_USER if scope == "user" else winreg.HKEY_LOCAL_MACHINE
    try:
        with winreg.OpenKey(hive, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as handle:
            values, index = {}, 0
            while True:
                try:
                    name, value, _type = winreg.EnumValue(handle, index)
                except OSError:
                    return values
                values[name] = value
                index += 1
    except FileNotFoundError:
        return None


def tree_hashes(folder):
    return {path.relative_to(folder).as_posix(): sha256_file(path)
            for path in sorted(Path(folder).rglob("*")) if path.is_file()}


class Run:
    def __init__(self, manifest, work, previous_folder, dry_run, ui_smoke, environment=os.environ,
                 out=sys.stdout):
        self.manifest = manifest
        self.work = Path(work)
        self.logs = self.work / "logs"
        self.dry_run = dry_run
        self.ui_smoke = ui_smoke
        self.environment = environment
        self.out = out
        self.counter = 0
        self.packages = {"new": manifest["new"], "older": manifest["older"]}
        self.versions = {"new": manifest["version"], "older": manifest["older"]["version"]}
        self.previous = []
        if previous_folder:
            for version in previous_versions(manifest["version"]):
                msi = Path(previous_folder) / f"v{version}" / f"midnight-explorer-{version}-{TARGET}.msi"
                if not msi.is_file() and not dry_run:
                    raise Failure(f"The published {version} MSI is missing: {msi} (run `previous` first)")
                self.packages[f"previous-{version}"] = {"msi": str(msi)}
                self.versions[f"previous-{version}"] = version
                self.previous.append(version)
        self.hashes = {}
        self.codes = {}
        self.marker = None

    def say(self, text):
        print(text, file=self.out, flush=True)

    # Facts about packages.

    def code(self, package):
        if package not in self.codes:
            self.codes[package] = package_property(self.packages[package]["msi"], "ProductCode")
        return self.codes[package]

    def payload(self, package):
        if package not in self.hashes:
            self.hashes[package] = payload_hashes(self.packages[package]["zip"])[1]
        return self.hashes[package]

    # msiexec.

    def msiexec(self, operation, package, scope=None):
        self.counter += 1
        if not self.dry_run:
            self.logs.mkdir(parents=True, exist_ok=True)
        log = self.logs / f"{self.counter:02d}-{operation}-{package}-{scope or 'default'}.log"
        arguments = msiexec_arguments(operation, self.packages[package]["msi"], log, scope)
        executable = str(Path(package_msi.system_tool("msiexec.exe"))) if not self.dry_run else "msiexec.exe"
        self.say("  > " + subprocess.list2cmdline([executable, *arguments]))
        if self.dry_run:
            return 0, ""
        completed = subprocess.run([executable, *arguments], timeout=MSIEXEC_TIMEOUT)
        text = decode_log(log.read_bytes()) if log.is_file() else ""
        self.say(f"    exit code {completed.returncode}")
        return completed.returncode, text

    def succeed(self, operation, package, scope=None):
        code, text = self.msiexec(operation, package, scope)
        if code not in SUCCESS:
            reason = next((line for line in text.splitlines() if "Return value 3" in line), "")
            raise Failure(f"msiexec {operation} of {package} ({scope or 'default'}) ended with {code} {reason}")

    # Checks.

    def installed_scopes(self, package):
        return [scope for scope in SCOPES if product_version(self.code(package), scope)]

    def check_installed(self, package, scope):
        version = self.versions[package]
        problems = []
        found = product_version(self.code(package), scope)
        if found != version:
            problems.append(f"Windows Installer reports {found!r} in the {scope} context, expected {version}")
        folder = install_folder(scope, self.environment)
        if not folder.is_dir():
            problems.append(f"{folder} is missing")
        else:
            problems += compare_files(self.payload(package), tree_hashes(folder))
        values = registry_values(scope, REGISTRY_KEY)
        if values is None or values.get("RootFolder") != 1 or values.get("StartMenuShortcut") != 1:
            problems.append(f"{REGISTRY_KEY} in {'HKCU' if scope == 'user' else 'HKLM'} is {values!r}")
        link = shortcut_path(scope, self.environment)
        if not link.is_file():
            problems.append(f"{link} is missing")
        else:
            problems += [f"shortcut: {problem}" for problem in check_shortcut(
                parse_shortcut(link.read_bytes()), folder / EXECUTABLE,
                expected_working_folder(scope, self.environment))]
        source, entry = entry_values(self.code(package), scope)
        if entry is None:
            problems.append(f"no Installed apps entry {self.code(package)} for the {scope} scope")
        else:
            problems += [f"Installed apps ({source}): {problem}" for problem in check_uninstall_entry(
                entry, version, folder, lambda path: Path(path).is_file(), registry=source == "registry")]
        if problems:
            raise Failure(f"{package} {version} ({scope}):\n  " + "\n  ".join(problems))
        self.say(f"    {package} {version} is installed for the {scope} scope: "
                 f"{len(self.payload(package))} files match, key, shortcut and entry checked")

    def check_removed(self, scope):
        problems = []
        for package in self.packages:
            if product_version(self.code(package), scope):
                problems.append(f"{package} is still installed in the {scope} context")
        folder = install_folder(scope, self.environment)
        if folder.exists():
            problems.append(f"{folder} is left: {sorted(tree_hashes(folder))[:10]}")
        if registry_values(scope, REGISTRY_KEY) is not None:
            problems.append(f"{REGISTRY_KEY} is left in {'HKCU' if scope == 'user' else 'HKLM'}")
        if shortcut_path(scope, self.environment).exists():
            problems.append(f"{shortcut_path(scope, self.environment)} is left")
        for package in self.packages:
            if registry_values(scope, f"{UNINSTALL_KEY}\\{self.code(package)}") is not None:
                problems.append(f"the Add/Remove Programs entry of {package} is left ({scope})")
        if problems:
            raise Failure(f"Removal ({scope}):\n  " + "\n  ".join(problems))
        self.say(f"    nothing of the {scope} installation is left")

    def check_replaced(self, package, scope):
        if product_version(self.code(package), scope):
            raise Failure(f"{package} is still installed in the {scope} context after the upgrade")
        if registry_values(scope, f"{UNINSTALL_KEY}\\{self.code(package)}") is not None:
            raise Failure(f"The Add/Remove Programs entry of {package} is left after the upgrade")
        self.say(f"    {package} was removed by the upgrade")

    def check_published(self, package):
        version = self.versions[package]
        if product_version(self.code(package), "user") != version:
            raise Failure(f"The published {version} MSI did not install for the current user")
        if not (install_folder("user", self.environment) / EXECUTABLE).is_file():
            raise Failure(f"The published {version} MSI did not install into the per-user MSI folder")
        self.say(f"    the published {version} MSI is installed in the per-user MSI folder")

    def smoke(self, scope, ui):
        executable = install_folder(scope, self.environment) / EXECUTABLE
        runs = [["--smoke-test"]] + ([["--ui-smoke-test", "--software"]] if ui else [])
        for arguments in runs:
            self.say("  > " + subprocess.list2cmdline([str(executable), *arguments]))
            if self.dry_run:
                continue
            completed = subprocess.run([str(executable), *arguments], cwd=executable.parent, capture_output=True,
                                       text=True, errors="replace", timeout=SMOKE_TIMEOUT,
                                       stdin=subprocess.DEVNULL)
            output = (completed.stdout + completed.stderr).strip()
            self.say("    " + output.replace("\n", "\n    ") if output else "    (no output)")
            if completed.returncode != 0:
                raise Failure(f"The installed executable ({scope}) {' '.join(arguments)} ended with "
                              f"{completed.returncode}")
            if arguments == ["--smoke-test"] and "installation: private" not in completed.stdout:
                raise Failure(f"The {scope} installation is not private to administrators and its user")

    def repair(self, package, scope):
        folder = install_folder(scope, self.environment)
        names = sorted(self.payload(package))
        deepest = max(names, key=lambda name: (name.count("/"), name))
        removed = sorted({EXECUTABLE, "README.txt", deepest})
        self.say(f"    deleting {', '.join(removed)}")
        if not self.dry_run:
            for name in removed:
                (folder / name).unlink()
        self.succeed("repair", package)

    def check_permissions(self, scope):
        folder = install_folder(scope, self.environment)
        program_files = Path(self.environment["ProgramW6432"])
        if self.dry_run:
            self.say(f"    compare the permissions of {folder} with those of {program_files}")
            return
        paths = [folder, *(path for path in sorted(folder.rglob("*")) if path.is_dir()),
                 *(path for path in sorted(folder.iterdir()) if path.is_file())]
        objects = read_permissions(paths)
        baseline = read_permissions([program_files])
        problems = write_grants(objects, baseline)
        if problems:
            raise Failure("The all-users folder has permissions beyond Program Files':\n  " + "\n  ".join(problems))
        self.say(f"    {len(paths)} folders and files: only administrators may change them, as in Program Files")

    def check_side_by_side(self):
        for scope in SCOPES:
            self.check_installed("new", scope)
        self.say("    both scopes are installed side by side with two Add/Remove Programs entries")

    def uninstall_one(self, package):
        before = self.installed_scopes(package) if not self.dry_run else []
        self.succeed("uninstall", package)
        if self.dry_run:
            return
        after = self.installed_scopes(package)
        removed = [scope for scope in before if scope not in after]
        if len(removed) != 1 or len(after) != len(before) - 1:
            raise Failure(f"One uninstall removed {removed} (installed before: {before}, after: {after})")
        self.check_removed(removed[0])
        for scope in after:
            self.check_installed(package, scope)

    def preflight(self):
        if self.dry_run:
            self.say("    nothing of Midnight Explorer may be installed; the per-machine steps need an "
                     "administrator")
            return
        if os.name != "nt":
            raise Failure("The MSI installation test needs Windows")
        if not ctypes.windll.shell32.IsUserAnAdmin():
            raise Failure("The MSI installation test needs an elevated administrator (per-machine installs)")
        related = related_products()
        if related:
            raise Failure(f"Refused: products of the Midnight Explorer MSI are installed: {related}")
        for scope in SCOPES:
            for path in (install_folder(scope, self.environment), shortcut_path(scope, self.environment)):
                if path.exists():
                    raise Failure(f"Refused: {path} exists")
            if registry_values(scope, REGISTRY_KEY) is not None:
                raise Failure(f"Refused: {REGISTRY_KEY} exists ({scope})")
        self.say("    nothing of the Midnight Explorer MSI is installed")

    def place_marker(self):
        folder = Path(self.environment["LOCALAPPDATA"]) / SETTINGS_FOLDER
        self.marker = (folder / MARKER, secrets.token_hex(16))
        self.say(f"    {self.marker[0]}")
        if not self.dry_run:
            folder.mkdir(exist_ok=True)
            self.marker[0].write_text(self.marker[1], encoding="ascii")

    def check_marker(self):
        if self.dry_run:
            return
        path, token = self.marker
        if not path.is_file() or path.read_text(encoding="ascii") != token:
            raise Failure(f"The settings folder was changed: {path}")
        self.say(f"    {path.parent} kept its marker file")

    def ui_install(self, package, scope):
        self.counter += 1
        if not self.dry_run:
            self.logs.mkdir(parents=True, exist_ok=True)
        stem = self.logs / f"{self.counter:02d}-ui-install-{package}-{scope}"
        command = ui_command(self.packages[package]["msi"], scope, stem.with_suffix(".log"),
                             stem.with_suffix(".png"))
        self.say("  > " + subprocess.list2cmdline(command))
        if self.dry_run:
            return
        completed = subprocess.run(command, timeout=MSIEXEC_TIMEOUT, env=windows_powershell_environment())
        if completed.returncode != 0:
            raise Failure(f"The full-UI installation ({scope}) ended with {completed.returncode}")

    def execute(self, steps):
        for operation, details in steps:
            self.say(f"- {operation} {json.dumps(details) if details else ''}".rstrip())
            package, scope = details.get("package"), details.get("scope")
            if operation == "preflight":
                self.preflight()
            elif operation == "marker":
                self.place_marker()
            elif operation == "install":
                self.succeed("install", package, scope)
            elif operation == "install-refused":
                code, text = self.msiexec("install", package, scope)
                if not self.dry_run and not downgrade_refused(code, text):
                    raise Failure(f"Installing {package} over a newer version was not refused (exit {code})")
            elif operation == "uninstall":
                self.succeed("uninstall", package)
            elif operation == "uninstall-one":
                self.uninstall_one(package)
            elif operation == "repair":
                self.repair(package, scope)
            elif operation == "smoke":
                self.smoke(scope, details["ui"])
            elif operation == "ui-install":
                self.ui_install(package, scope)
            elif self.dry_run:
                continue
            elif operation == "check-installed":
                self.check_installed(package, scope)
            elif operation == "check-replaced":
                self.check_replaced(package, scope)
            elif operation == "check-published":
                self.check_published(package)
            elif operation == "check-removed":
                self.check_removed(scope)
            elif operation == "check-permissions":
                self.check_permissions(scope)
            elif operation == "check-side-by-side":
                self.check_side_by_side()
            elif operation == "check-marker":
                self.check_marker()
            elif operation == "check-nothing-installed":
                for scope in SCOPES:
                    self.check_removed(scope)
                if related_products():
                    raise Failure(f"Products are left: {related_products()}")
            else:
                raise ValueError(f"Unknown step {operation}")


def ui_command(msi, scope, log, screenshot):
    return ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File",
            str(ROOT / "scripts" / "msi-ui-install.ps1"), "-Msi", str(msi), "-Scope", scope,
            "-Log", str(log), "-Screenshot", str(screenshot)]


def windows_powershell_environment(**values):
    """The environment for Windows PowerShell 5.1, without PowerShell 7's module path
    (which stops it from loading its own modules when started from pwsh)."""
    environment = {name: value for name, value in os.environ.items() if name.upper() != "PSMODULEPATH"}
    environment.update(values)
    return environment


# The security descriptor through .NET, so that no PowerShell module needs to load.
ACL_SCRIPT = r"""
$ErrorActionPreference = 'Stop'
$sid = [Security.Principal.SecurityIdentifier]
$sections = [Security.AccessControl.AccessControlSections]'Access, Owner'
$result = foreach ($path in ($env:ME_ACL_PATHS -split "`n")) {
  $acl = if ([IO.Directory]::Exists($path)) { New-Object Security.AccessControl.DirectorySecurity($path, $sections) }
         else { New-Object Security.AccessControl.FileSecurity($path, $sections) }
  [pscustomobject]@{
    Path = $path
    Owner = $acl.GetOwner($sid).Value
    Rules = @($acl.GetAccessRules($true, $true, $sid) | ForEach-Object {
      [pscustomobject]@{ Sid = $_.IdentityReference.Value; Rights = [int64]$_.FileSystemRights
        Allow = ($_.AccessControlType -eq 'Allow'); InheritOnly = (($_.PropagationFlags -band 2) -ne 0) } })
  }
}
ConvertTo-Json -InputObject @($result) -Depth 5 -Compress
"""


def read_permissions(paths):
    environment = windows_powershell_environment(ME_ACL_PATHS="\n".join(str(path) for path in paths))
    completed = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", ACL_SCRIPT],
                               capture_output=True, text=True, env=environment, timeout=300)
    if completed.returncode != 0:
        raise Failure(f"Reading permissions failed: {completed.stderr}")
    return json.loads(completed.stdout)


# Published MSIs ---------------------------------------------------------------------------


def download_previous(repository, output, out=sys.stdout):
    """The published per-user MSIs older than the workspace version, each checked against
    its release's SHA256SUMS and that file's signature (packaging/update-signing.pub)."""
    signing = _load("release_signing", "release-signing.py")
    output = package_release.build_output(output, ROOT)
    current = package_msi.workspace_version()
    for version in previous_versions(current):
        folder = output / f"v{version}"
        if folder.exists():
            raise Failure(f"Refused: {folder} exists")
        folder.mkdir(parents=True)
        name = f"midnight-explorer-{version}-{TARGET}.msi"
        command = ["gh", "release", "download", f"v{version}", "--repo", repository, "--dir", str(folder),
                   "--pattern", name, "--pattern", "SHA256SUMS", "--pattern", "SHA256SUMS.minisig"]
        print("> " + subprocess.list2cmdline(command), file=out, flush=True)
        subprocess.run(command, check=True, timeout=600)
        sums = folder / "SHA256SUMS"
        public = signing.read_public_key(signing.PUBLIC_KEY.read_text(encoding="utf-8"))
        try:
            signing.verify_signature(public, sums.read_bytes(),
                                     (folder / "SHA256SUMS.minisig").read_text(encoding="ascii"), version)
        except ValueError as error:
            raise Failure(f"The {version} SHA256SUMS signature is refused: {error}") from None
        listed = dict(reversed(line.split(None, 1)) for line in sums.read_text(encoding="ascii").splitlines()
                      if line.strip())
        if listed.get(name) != sha256_file(folder / name):
            raise Failure(f"{name} does not match the signed SHA256SUMS")
        print(f"{name}: matches the signed SHA256SUMS of {version}", file=out)


# ICE validation ---------------------------------------------------------------------------


def find_validation_package(environment=os.environ):
    """The newest Windows SDK MsiVal2 package, which holds darice.cub, or None."""
    kits = Path(environment.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits" / "10" / "bin"
    candidates = []
    for path in kits.glob("10.*/x86/MsiVal2-x86_en-us.msi"):
        try:
            candidates.append((tuple(int(part) for part in path.parts[-3].split(".")), path))
        except ValueError:
            continue
    return max(candidates)[1] if candidates else None


def extract_cub(validation_package, folder):
    """darice.cub from the SDK's MsiVal2 package and its cabinet beside it (nothing installed)."""
    import msi_database
    with msi_database.Database(validation_package) as database:
        files = [row for row in database.rows("SELECT `File`, `FileName`, `Sequence` FROM `File`")
                 if row[1].split("|")[-1].lower() == "darice.cub"]
        media = database.rows("SELECT `DiskId`, `LastSequence`, `Cabinet` FROM `Media`")
    if len(files) != 1:
        raise Failure(f"{validation_package} has no darice.cub")
    file_key, _name, sequence = files[0]
    cabinet = next(row[2] for row in sorted(media, key=lambda row: row[1]) if row[1] >= sequence)
    if not cabinet or cabinet.startswith("#"):
        raise Failure(f"darice.cub of {validation_package} is not in an external cabinet")
    folder.mkdir(parents=True)
    completed = subprocess.run([str(package_msi.system_tool("expand.exe")), str(validation_package.parent / cabinet),
                                f"-F:{file_key}", str(folder)], capture_output=True, text=True, errors="replace")
    cub = folder / file_key
    if completed.returncode != 0 or not cub.is_file():
        raise Failure(f"expand.exe could not extract darice.cub: {completed.stdout}{completed.stderr}")
    return cub


def run_ices(msi_path, cub, work):
    """The ICE messages of `msi_path`: a copy with darice.cub merged into it, whose ICE
    actions run in a client session (the way msival2 and Orca validate; nothing installs)."""
    import msi_database
    msi = msi_database._library()
    copy = work / "validate.msi"
    shutil.copyfile(msi_path, copy)
    msi.MsiDatabaseMergeW.argtypes = (msi_database.MSIHANDLE, msi_database.MSIHANDLE, wintypes.LPCWSTR)
    msi.MsiDatabaseMergeW.restype = wintypes.UINT
    with msi_database.Database(copy, update=True) as database, msi_database.Database(cub) as rules:
        # Rows that both define (Property) are reported in _MergeErrors and kept as the package's.
        result = msi.MsiDatabaseMergeW(database._handle.value, rules._handle.value, "_MergeErrors")
        if result not in (0, 1627):
            raise Failure(f"Merging darice.cub failed with {result}")
        database.execute("DROP TABLE `_MergeErrors`")
        database.commit()
    with msi_database.Database(copy) as database:
        actions = [row[0] for row in sorted(database.rows("SELECT `Action`, `Sequence` FROM `_ICESequence`"),
                                            key=lambda row: row[1])]
    messages = []
    handler_type = ctypes.WINFUNCTYPE(ctypes.c_int, ctypes.c_void_p, wintypes.UINT, wintypes.LPCWSTR)

    def handler(_context, kind, text):
        if kind & 0xFF000000 == 0x03000000 and text:  # INSTALLMESSAGE_USER
            messages.append(text)
            return 1  # IDOK
        return 0

    callback = handler_type(handler)
    msi.MsiSetExternalUIW.argtypes = (handler_type, wintypes.DWORD, ctypes.c_void_p)
    msi.MsiSetExternalUIW.restype = ctypes.c_void_p
    msi.MsiSetInternalUI.argtypes = (wintypes.UINT, ctypes.c_void_p)
    msi.MsiSetInternalUI.restype = wintypes.UINT
    msi.MsiOpenPackageW.argtypes = (wintypes.LPCWSTR, ctypes.POINTER(msi_database.MSIHANDLE))
    msi.MsiOpenPackageW.restype = wintypes.UINT
    msi.MsiDoActionW.argtypes = (msi_database.MSIHANDLE, wintypes.LPCWSTR)
    msi.MsiDoActionW.restype = wintypes.UINT
    previous_ui = msi.MsiSetInternalUI(2, None)  # INSTALLUILEVEL_NONE
    msi.MsiSetExternalUIW(callback, 1 << 3, None)  # INSTALLLOGMODE_USER
    try:
        handle = msi_database.MSIHANDLE()
        result = msi.MsiOpenPackageW(str(copy), ctypes.byref(handle))
        if result != 0:
            raise Failure(f"Opening {copy.name} for validation failed with {result}")
        try:
            failed = [(action, code) for action in actions
                      if (code := msi.MsiDoActionW(handle.value, action)) not in (0, ERROR_MORE_DATA)]
        finally:
            msi.MsiCloseHandle(handle.value)
    finally:
        msi.MsiSetExternalUIW(handler_type(), 0, None)
        msi.MsiSetInternalUI(previous_ui, None)
    return actions, messages, failed


def validate(msi_path, work, out=sys.stdout):
    """ICE validation of `msi_path`; False when the Windows SDK's rules are not installed."""
    package = find_validation_package()
    if package is None:
        print("::warning::ICE validation skipped: no Windows SDK MsiVal2 package (darice.cub) is "
              "installed on this machine", file=out)
        return False
    work = package_release.build_output(work, ROOT)
    if work.exists():
        raise Failure(f"Refused: {work} exists")
    work.mkdir(parents=True)
    cub = extract_cub(package, work / "cub")
    actions, messages, failed = run_ices(Path(msi_path), cub, work)
    unexpected, recorded, warnings = ice_findings(messages)
    print(f"{len(actions)} ICEs of {package} ran on {Path(msi_path).name}", file=out)
    for (ice, text), count in sorted(known_counts(recorded).items()):
        print(f"  known (REL-05): {ice} {text} ({count}x)", file=out)
    for ice, kind, description in warnings:
        print(f"  warning: {ice} {description}", file=out)
    for ice, kind, description in unexpected:
        print(f"  {kind}: {ice} {description}", file=out)
    if failed:
        raise Failure(f"ICE actions could not run: {failed}")
    if unexpected:
        raise Failure(f"{len(unexpected)} ICE errors beyond the recorded findings")
    return True


# Command line -----------------------------------------------------------------------------


def main(argv=None, out=sys.stdout):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    made = commands.add_parser("stand-in", help="a release-shaped ZIP of a release build")
    made.add_argument("--executable", required=True)
    made.add_argument("--helper", required=True)
    made.add_argument("--viewers", required=True)
    made.add_argument("--filesystems", required=True)
    made.add_argument("--output", required=True)
    prepared = commands.add_parser("prepare", help="the MSIs of a release ZIP and of an older copy")
    prepared.add_argument("--zip", required=True)
    prepared.add_argument("--work", required=True)
    downloaded = commands.add_parser("previous", help="download and check the published per-user MSIs")
    downloaded.add_argument("--repository", required=True, help="owner/name on GitHub")
    downloaded.add_argument("--output", required=True)
    checked = commands.add_parser("ice", help="Windows SDK ICE validation")
    checked.add_argument("--msi", required=True)
    checked.add_argument("--work", required=True)
    tested = commands.add_parser("run", help="install, upgrade, repair and remove the MSI")
    tested.add_argument("--work", required=True, help="the folder that `prepare` made")
    tested.add_argument("--previous", help="the folder that `previous` made")
    tested.add_argument("--scenario", action="append", choices=SCENARIOS,
                        help="scopes (default), previous (needs --previous) or ui (full UI)")
    tested.add_argument("--ui-smoke", action="store_true", help="also start the installed UI (needs a desktop)")
    tested.add_argument("--disposable-machine", action="store_true",
                        help="confirm that this machine is a throwaway (CI); required unless --dry-run")
    tested.add_argument("--dry-run", action="store_true", help="print the plan; change nothing")
    args = parser.parse_args(argv)
    try:
        if args.command == "stand-in":
            stand_in(args.executable, args.helper, args.viewers, args.filesystems, args.output, out=out)
        elif args.command == "prepare":
            prepare(args.zip, args.work, out=out)
        elif args.command == "previous":
            download_previous(args.repository, args.output, out=out)
        elif args.command == "ice":
            validate(args.msi, args.work, out=out)
        else:
            refuse_unless_disposable(args.disposable_machine, args.dry_run)
            work = package_release.build_output(args.work, ROOT)
            manifest = json.loads((work / "manifest.json").read_text(encoding="utf-8"))
            scenarios = args.scenario or ["scopes"]
            if "previous" in scenarios and not args.previous:
                raise Failure("The previous scenario needs --previous")
            folder = ("dry-run-" if args.dry_run else "run-") + "-".join(scenarios)
            run = Run(manifest, work / folder,
                      args.previous if "previous" in scenarios else None, args.dry_run, args.ui_smoke, out=out)
            steps = plan(scenarios, manifest["version"], run.previous, ui_smoke=args.ui_smoke)
            run.execute(steps)
            print(("Planned" if args.dry_run else "Passed") + f": {len(steps)} steps of {', '.join(scenarios)}",
                  file=out)
    except (Failure, ValueError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAILED: {error}") from None
    return 0


if __name__ == "__main__":
    sys.exit(main())
