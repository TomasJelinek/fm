#!/usr/bin/env python3
"""Build and check the Windows installer (MSI) of a finished release ZIP.

The MSI is made from the release ZIP itself, the single source of its files, with the
Windows Installer API (msi.dll) and the cabinet tools (makecab.exe, expand.exe) that
Windows ships; no third-party installer toolkit is used. It never builds the
application and never publishes anything.
"""

import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
import uuid
import zipfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("package_release", ROOT / "scripts/package-release.py")
package_release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(package_release)

TARGET = "x86_64-pc-windows-msvc"
PRODUCT = "Midnight Explorer"
PUBLISHER = "Tomas Jelinek"
EXECUTABLE = "midnight-explorer.exe"
SOURCE_REPOSITORY = package_release.SOURCE_REPOSITORY
# Never change: Windows Installer recognises every later MSI of the product by it.
UPGRADE_CODE = "{DDF2A2BA-6901-484C-97B2-4EF662BF5E72}"
# Namespace of the name-based (version 5) product, package and component codes, so the
# same release ZIP always gives the same codes.
CODE_NAMESPACE = uuid.UUID("7d5101d9-ee9a-4b46-aa85-1cab4fce4878")
# One dual-purpose package (ALLUSERS=2 with MSIINSTALLPERUSER): "Just for me" installs into
# %LOCALAPPDATA%\Programs\<INSTALL_FOLDER> without administrator rights (the default, and
# the folder of the earlier per-user MSI, which it upgrades in place); "For all users"
# installs into %ProgramFiles%\<INSTALL_FOLDER>, where Windows Installer asks for UAC
# consent. Not "Midnight Explorer": per user that folder is the destination of the
# application's own "Move to my profile..." copy, and below Program Files it is the one the
# user guide names for an administrator's copy of a build; an installer must neither
# overwrite them nor have its files replaced under it.
INSTALL_FOLDER = "Midnight Explorer (MSI)"
USER_INSTALL_FOLDER = f"[LocalAppDataFolder]Programs\\{INSTALL_FOLDER}\\"
# Program Files of 64-bit Windows, for the dialog's choice: in a per-user session
# ProgramFiles64Folder already names the per-user folder.
MACHINE_INSTALL_FOLDER = f"[%ProgramW6432]\\{INSTALL_FOLDER}\\"
# The registry key that holds the components' key paths, removed on uninstall.
REGISTRY_KEY = r"Software\Midnight Explorer Installer"
# The application's updater replaces only a folder with BUILD-INFO.json ("a package that
# another installer manages has none"), so an installed MSI is updated by a newer MSI.
EXCLUDED = ("BUILD-INFO.json",)
CABINET = "payload.cab"
ICON = ROOT / "assets/branding/midnight-explorer.ico"
MAXIMUM_FILES = 65536
MAXIMUM_PAYLOAD = 2 * 1024 ** 3 - 1
# Windows' 260-character path limit: with a profile such as C:\Users\<20 characters>,
# INSTALLDIR is 77 characters, so payload paths of at most 160 leave a margin.
MAXIMUM_RELATIVE_PATH = 160
ARCHIVE_NAME = re.compile(r"midnight-explorer-(\d+)\.(\d+)\.(\d+)-x86_64-pc-windows-msvc\.zip")

# Word count: compressed files (2) and no elevated privileges needed (8) for the default
# per-user installation; choosing all users makes Windows Installer ask for UAC consent.
WORD_COUNT = 10
SCHEMA = 500

# Columns follow the Windows Installer database schema; only the tables used are made.
TABLES = {
    "Property": "`Property` CHAR(72) NOT NULL, `Value` CHAR(0) NOT NULL LOCALIZABLE PRIMARY KEY `Property`",
    "Directory": ("`Directory` CHAR(72) NOT NULL, `Directory_Parent` CHAR(72), "
                  "`DefaultDir` CHAR(255) NOT NULL LOCALIZABLE PRIMARY KEY `Directory`"),
    "Component": ("`Component` CHAR(72) NOT NULL, `ComponentId` CHAR(38), `Directory_` CHAR(72) NOT NULL, "
                  "`Attributes` SHORT NOT NULL, `Condition` CHAR(255), `KeyPath` CHAR(72) "
                  "PRIMARY KEY `Component`"),
    "Feature": ("`Feature` CHAR(38) NOT NULL, `Feature_Parent` CHAR(38), `Title` CHAR(64) LOCALIZABLE, "
                "`Description` CHAR(255) LOCALIZABLE, `Display` SHORT, `Level` SHORT NOT NULL, "
                "`Directory_` CHAR(72), `Attributes` SHORT NOT NULL PRIMARY KEY `Feature`"),
    "FeatureComponents": ("`Feature_` CHAR(38) NOT NULL, `Component_` CHAR(72) NOT NULL "
                          "PRIMARY KEY `Feature_`, `Component_`"),
    "File": ("`File` CHAR(72) NOT NULL, `Component_` CHAR(72) NOT NULL, `FileName` CHAR(255) NOT NULL "
             "LOCALIZABLE, `FileSize` LONG NOT NULL, `Version` CHAR(72), `Language` CHAR(20), "
             "`Attributes` SHORT, `Sequence` LONG NOT NULL PRIMARY KEY `File`"),
    "Media": ("`DiskId` SHORT NOT NULL, `LastSequence` LONG NOT NULL, `DiskPrompt` CHAR(64) LOCALIZABLE, "
              "`Cabinet` CHAR(255), `VolumeLabel` CHAR(32), `Source` CHAR(72) PRIMARY KEY `DiskId`"),
    "Registry": ("`Registry` CHAR(72) NOT NULL, `Root` SHORT NOT NULL, `Key` CHAR(255) NOT NULL "
                 "LOCALIZABLE, `Name` CHAR(255) LOCALIZABLE, `Value` CHAR(0) LOCALIZABLE, "
                 "`Component_` CHAR(72) NOT NULL PRIMARY KEY `Registry`"),
    "RemoveFile": ("`FileKey` CHAR(72) NOT NULL, `Component_` CHAR(72) NOT NULL, `FileName` CHAR(255) "
                   "LOCALIZABLE, `DirProperty` CHAR(72) NOT NULL, `InstallMode` SHORT NOT NULL "
                   "PRIMARY KEY `FileKey`"),
    "Shortcut": ("`Shortcut` CHAR(72) NOT NULL, `Directory_` CHAR(72) NOT NULL, `Name` CHAR(128) NOT NULL "
                 "LOCALIZABLE, `Component_` CHAR(72) NOT NULL, `Target` CHAR(72) NOT NULL, "
                 "`Arguments` CHAR(255), `Description` CHAR(255) LOCALIZABLE, `Hotkey` SHORT, "
                 "`Icon_` CHAR(72), `IconIndex` SHORT, `ShowCmd` SHORT, `WkDir` CHAR(72) "
                 "PRIMARY KEY `Shortcut`"),
    "Icon": "`Name` CHAR(72) NOT NULL, `Data` OBJECT NOT NULL PRIMARY KEY `Name`",
    "Upgrade": ("`UpgradeCode` CHAR(38) NOT NULL, `VersionMin` CHAR(20), `VersionMax` CHAR(20), "
                "`Language` CHAR(255), `Attributes` LONG NOT NULL, `Remove` CHAR(255), "
                "`ActionProperty` CHAR(72) NOT NULL PRIMARY KEY `UpgradeCode`, `VersionMin`, "
                "`VersionMax`, `Language`, `Attributes`"),
    "LaunchCondition": ("`Condition` CHAR(255) NOT NULL, `Description` CHAR(255) NOT NULL LOCALIZABLE "
                        "PRIMARY KEY `Condition`"),
    "CustomAction": ("`Action` CHAR(72) NOT NULL, `Type` SHORT NOT NULL, `Source` CHAR(72), "
                     "`Target` CHAR(255) PRIMARY KEY `Action`"),
    "AppSearch": "`Property` CHAR(72) NOT NULL, `Signature_` CHAR(72) NOT NULL PRIMARY KEY `Property`, `Signature_`",
    # The AppSearch action reads Signature for every search, also when only registry
    # locators are used: without the (empty) table it fails with error 2228.
    "Signature": ("`Signature` CHAR(72) NOT NULL, `FileName` CHAR(255) NOT NULL, `MinVersion` CHAR(20), "
                  "`MaxVersion` CHAR(20), `MinSize` LONG, `MaxSize` LONG, `MinDate` LONG, `MaxDate` LONG, "
                  "`Languages` CHAR(255) PRIMARY KEY `Signature`"),
    "RegLocator": ("`Signature_` CHAR(72) NOT NULL, `Root` SHORT NOT NULL, `Key` CHAR(255) NOT NULL, "
                   "`Name` CHAR(255), `Type` SHORT PRIMARY KEY `Signature_`"),
    "TextStyle": ("`TextStyle` CHAR(72) NOT NULL, `FaceName` CHAR(32) NOT NULL, `Size` SHORT NOT NULL, "
                  "`Color` LONG, `StyleBits` SHORT PRIMARY KEY `TextStyle`"),
    "Dialog": ("`Dialog` CHAR(72) NOT NULL, `HCentering` SHORT NOT NULL, `VCentering` SHORT NOT NULL, "
               "`Width` SHORT NOT NULL, `Height` SHORT NOT NULL, `Attributes` LONG, `Title` CHAR(128) "
               "LOCALIZABLE, `Control_First` CHAR(50) NOT NULL, `Control_Default` CHAR(50), "
               "`Control_Cancel` CHAR(50) PRIMARY KEY `Dialog`"),
    "Control": ("`Dialog_` CHAR(72) NOT NULL, `Control` CHAR(50) NOT NULL, `Type` CHAR(20) NOT NULL, "
                "`X` SHORT NOT NULL, `Y` SHORT NOT NULL, `Width` SHORT NOT NULL, `Height` SHORT NOT NULL, "
                "`Attributes` LONG, `Property` CHAR(72), `Text` CHAR(0) LOCALIZABLE, `Control_Next` CHAR(50), "
                "`Help` CHAR(50) LOCALIZABLE PRIMARY KEY `Dialog_`, `Control`"),
    "ControlEvent": ("`Dialog_` CHAR(72) NOT NULL, `Control_` CHAR(50) NOT NULL, `Event` CHAR(50) NOT NULL, "
                     "`Argument` CHAR(255) NOT NULL, `Condition` CHAR(255), `Ordering` SHORT "
                     "PRIMARY KEY `Dialog_`, `Control_`, `Event`, `Argument`, `Condition`"),
    "ControlCondition": ("`Dialog_` CHAR(72) NOT NULL, `Control_` CHAR(50) NOT NULL, `Action` CHAR(50) NOT NULL, "
                         "`Condition` CHAR(255) NOT NULL PRIMARY KEY `Dialog_`, `Control_`, `Action`, `Condition`"),
    "EventMapping": ("`Dialog_` CHAR(72) NOT NULL, `Control_` CHAR(50) NOT NULL, `Event` CHAR(50) NOT NULL, "
                     "`Attribute` CHAR(50) NOT NULL PRIMARY KEY `Dialog_`, `Control_`, `Event`"),
    "RadioButton": ("`Property` CHAR(72) NOT NULL, `Order` SHORT NOT NULL, `Value` CHAR(64) NOT NULL, "
                    "`X` SHORT NOT NULL, `Y` SHORT NOT NULL, `Width` SHORT NOT NULL, `Height` SHORT NOT NULL, "
                    "`Text` CHAR(0) LOCALIZABLE, `Help` CHAR(50) LOCALIZABLE PRIMARY KEY `Property`, `Order`"),
}
SEQUENCE_COLUMNS = ("`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT "
                    "PRIMARY KEY `Action`")
# Conditions of the installation scope. Windows Installer installs per user when ALLUSERS
# is empty or when ALLUSERS=2 and MSIINSTALLPERUSER=1, and per machine otherwise.
PER_USER = "NOT ALLUSERS OR (ALLUSERS = 2 AND MSIINSTALLPERUSER = 1)"
PER_MACHINE = "ALLUSERS = 1 OR (ALLUSERS = 2 AND NOT MSIINSTALLPERUSER)"
# One scope at a time. Windows Installer upgrades only a copy of the scope being installed,
# and copies in both scopes register the same component codes (they follow the files), so
# removing one of two copies would leave its files. A scope is therefore refused while the
# other scope has a copy: AppSearch finds it by the RootFolder value that every MSI of the
# product (0.6.1 on) writes under REGISTRY_KEY, in HKCU (ME_USER_COPY) or HKLM
# (ME_MACHINE_COPY). An upgrade in the same scope and the maintenance of the installed
# version (Installed) are not refused. The launch conditions refuse only in the execute
# sequence, where SetScopeRule sets SCOPE_RULE: the UI sequence evaluates them before
# ScopeDlg, still with the default scope, and there the dialog disables the conflicting
# choice's Install button instead.
SCOPE_RULE = "ME_SCOPE_RULE"
OTHER_SCOPE_REFUSALS = {
    "user": (f"{PRODUCT} is already installed for all users. Remove it in Settings > Apps > "
             "Installed apps before installing it just for you."),
    "machine": (f"{PRODUCT} is already installed just for you. Remove it in Settings > Apps > "
                "Installed apps before installing it for all users."),
}
OTHER_SCOPE_CONDITIONS = {
    "user": f"Installed OR NOT {SCOPE_RULE} OR NOT ME_MACHINE_COPY OR NOT ({PER_USER})",
    "machine": f"Installed OR NOT {SCOPE_RULE} OR NOT ME_USER_COPY OR ({PER_USER})",
}
# Standard action order (the numbers Windows Installer's documentation suggests), with an
# optional condition. The only custom actions only set properties (type 51); nothing runs
# code. The dialogs are the ones below (DIALOGS); the negative numbers are the dialogs
# shown at the end: success, cancelled and failed.
SEQUENCES = {
    "InstallUISequence": (
        ("FindRelatedProducts", 25), ("AppSearch", 50), ("LaunchConditions", 100),
        ("ValidateProductID", 700), ("CostInitialize", 800), ("FileCost", 900),
        ("SetUserInstallFolder", 990, f"NOT INSTALLDIR AND ({PER_USER})"),
        ("CostFinalize", 1000),
        ("SetMachineScope", 1100, f"{PER_MACHINE} OR (ME_MACHINE_COPY AND NOT ME_USER_COPY)"),
        ("MigrateFeatureStates", 1200), ("ScopeDlg", 1230, "NOT Installed"),
        ("MaintenanceDlg", 1250, "Installed AND NOT RESUME AND NOT Preselected"),
        ("ProgressDlg", 1280), ("ExecuteAction", 1300),
        ("ExitDlg", -1), ("UserExitDlg", -2), ("FatalErrorDlg", -3)),
    "InstallExecuteSequence": (
        # AppSearch here too: a silent installation runs no UI sequence.
        ("FindRelatedProducts", 25), ("AppSearch", 50), ("SetScopeRule", 60),
        ("LaunchConditions", 100), ("ValidateProductID", 700),
        ("CostInitialize", 800), ("FileCost", 900),
        ("SetUserInstallFolder", 990, f"NOT INSTALLDIR AND ({PER_USER})"),
        ("CostFinalize", 1000),
        ("SetInstallLocation", 1001), ("SetShortcutWorkingFolder", 1002, PER_USER),
        ("SetMachineShortcutWorkingFolder", 1003, f"NOT ({PER_USER})"),
        ("MigrateFeatureStates", 1200), ("InstallValidate", 1400),
        # Early: the earlier version is removed completely before this one is installed.
        ("RemoveExistingProducts", 1401), ("InstallInitialize", 1500),
        ("ProcessComponents", 1600), ("UnpublishFeatures", 1800),
        ("RemoveRegistryValues", 2600), ("RemoveShortcuts", 3200), ("RemoveFiles", 3500),
        ("RemoveFolders", 3600), ("CreateFolders", 3700), ("InstallFiles", 4000),
        ("CreateShortcuts", 4500), ("WriteRegistryValues", 5000), ("RegisterUser", 6000),
        ("RegisterProduct", 6100), ("PublishFeatures", 6300), ("PublishProduct", 6400),
        ("InstallFinalize", 6600)),
    "AdminUISequence": (
        ("CostInitialize", 800), ("FileCost", 900), ("CostFinalize", 1000), ("ExecuteAction", 1300)),
    "AdminExecuteSequence": (
        ("CostInitialize", 800), ("FileCost", 900), ("CostFinalize", 1000),
        ("InstallValidate", 1400), ("InstallInitialize", 1500), ("InstallAdminPackage", 3900),
        ("InstallFiles", 4000), ("InstallFinalize", 6600)),
}
CUSTOM_ACTION_SET_PROPERTY = 51
# Add/Remove Programs shows the install folder. The Start menu shortcut's start folder is
# where the left panel opens on a first start: per user the installing user's profile, as
# the Desktop shortcut of scripts/windows-shortcut.ps1 does; for all users the Windows
# drive's root, which every account can open. A profile variable cannot serve all users:
# Windows Installer expands it when it creates the shortcut, so every account would start
# in the installing administrator's profile, and Windows refuses to start a shortcut whose
# start folder it cannot open. A per-user installation goes to the earlier per-user MSI's
# folder (Directory: Program Files), and the dialog's default scope follows the command
# line or an existing all-users copy. SetScopeRule, in the execute sequence only, lets the
# launch conditions refuse the other scope (OTHER_SCOPE_CONDITIONS).
CUSTOM_ACTIONS = (
    ("SetInstallLocation", CUSTOM_ACTION_SET_PROPERTY, "ARPINSTALLLOCATION", "[INSTALLDIR]"),
    ("SetShortcutWorkingFolder", CUSTOM_ACTION_SET_PROPERTY, "MESTARTFOLDER", "[%USERPROFILE]"),
    ("SetMachineShortcutWorkingFolder", CUSTOM_ACTION_SET_PROPERTY, "MESTARTFOLDER", "[WindowsVolume]"),
    ("SetUserInstallFolder", CUSTOM_ACTION_SET_PROPERTY, "INSTALLDIR", USER_INSTALL_FOLDER),
    ("SetMachineScope", CUSTOM_ACTION_SET_PROPERTY, "ME_SCOPE", "machine"),
    ("SetScopeRule", CUSTOM_ACTION_SET_PROPERTY, SCOPE_RULE, "1"),
)
# Registry root -1: HKCU in a per-user installation, HKLM in a per-machine one.
USER_OR_MACHINE = -1
HKCU = 1
HKLM = 2
REGISTRY_RAW_64BIT = 2 | 16
# Control attributes.
VISIBLE, ENABLED, TRANSPARENT, NO_PREFIX = 1, 2, 0x10000, 0x20000
PROGRESS_95 = 0x10000
ELEVATION_SHIELD = 0x800000
TEXT = VISIBLE | ENABLED | TRANSPARENT | NO_PREFIX
HIDDEN_TEXT = ENABLED | TRANSPARENT | NO_PREFIX
CONTROL = VISIBLE | ENABLED
DIALOG_MODAL = 1 | 2 | 4  # visible, modal, minimizable
DIALOG_MODELESS = 1 | 4
WIDTH, HEIGHT = 370, 270
COMPONENT_REGISTRY_KEY_PATH = 4
COMPONENT_64BIT = 256
FILE_VITAL = 512
UPGRADE_MIGRATE_FEATURES = 1
UPGRADE_ONLY_DETECT = 2
REMOVE_ON_UNINSTALL = 2


@dataclass(frozen=True)
class Payload:
    archive: Path
    version: str
    root: str
    files: tuple  # (relative path, ZipInfo), sorted; EXCLUDED files left out
    archive_sha256: str


def code(name):
    return "{" + str(uuid.uuid5(CODE_NAMESPACE, name)).upper() + "}"


def product_code(version):
    return code(f"product {version}")


def package_code(version, archive_sha256):
    return code(f"package {version} {archive_sha256}")


def key(prefix, name):
    """A stable Windows Installer identifier for a payload path."""
    return prefix + hashlib.sha256(name.encode("utf-8")).hexdigest()[:32]


def workspace_version(root=ROOT):
    with open(root / "Cargo.toml", "rb") as stream:
        return tomllib.load(stream)["workspace"]["package"]["version"]


def short_name(name, taken):
    """An 8.3 name for `name`, unique (case-insensitively) among `taken`, which it joins."""
    cleaned = "".join(character for character in name.replace("+", "_")
                      if character not in ' "/\\[]:;=,')
    parts = cleaned.split(".")
    if len(parts) > 1:
        prefix, suffix = "".join(parts[:-1]).upper(), parts[-1].upper()
        if not prefix:
            prefix, suffix = suffix, None
    else:
        prefix, suffix = cleaned.upper(), None
    candidate = None
    if (len(parts) < 3 and cleaned == name and not name.startswith(".") and 0 < len(prefix) <= 8
            and (suffix is None or 0 < len(suffix) <= 3)):
        candidate = prefix + ("." + suffix if suffix else "")
    if candidate is None or candidate in taken:
        prefix = (prefix or "_")[:6]
        suffix = suffix[:3] if suffix else None
        for number in range(1, 1000000):
            stem = prefix[:max(1, 7 - len(str(number)))]
            candidate = f"{stem}~{number}" + ("." + suffix if suffix else "")
            if candidate not in taken:
                break
        else:
            raise ValueError(f"No short name left for {name}")
    taken.add(candidate)
    return candidate


def msi_name(name, taken):
    """The Filename value `SHORT|long` (or the name alone when it is its own short name)."""
    short = short_name(name, taken)
    return short if short == name else f"{short}|{name}"


def read_payload(archive, *, expected_version=None):
    """The release ZIP's files, checked as the packager writes them."""
    archive = Path(archive)
    match = ARCHIVE_NAME.fullmatch(archive.name)
    if not match:
        raise ValueError(f"Not a Windows release archive name: {archive.name}")
    major, minor, patch = (int(part) for part in match.groups())
    if major > 255 or minor > 255 or patch > 65535:
        raise ValueError("Windows Installer versions are at most 255.255.65535")
    version = f"{major}.{minor}.{patch}"
    if expected_version is not None and version != expected_version:
        raise ValueError(f"The archive holds {version}, but the workspace version is {expected_version}")
    root = archive.name.removesuffix(".zip")
    digest = hashlib.sha256()
    with open(archive, "rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    files = []
    seen = set()
    total = 0
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        if len(entries) > MAXIMUM_FILES:
            raise ValueError(f"The archive has more than {MAXIMUM_FILES} entries")
        for info in entries:
            name = info.filename
            if info.is_dir() or info.external_attr >> 16 & 0o170000 not in (0, 0o100000):
                raise ValueError(f"Only regular files may be in the archive: {name}")
            path = PurePosixPath(name)
            if path.parts[0] != root or len(path.parts) < 2 or "\\" in name or not name.isascii():
                raise ValueError(f"Archive entry outside {root}/ or not plain ASCII: {name!r}")
            for part in path.parts:
                package_release.portable_component(part)
            relative = PurePosixPath(*path.parts[1:]).as_posix()
            if len(relative) > MAXIMUM_RELATIVE_PATH:
                raise ValueError(f"Path longer than {MAXIMUM_RELATIVE_PATH} characters: {relative}")
            folded = relative.casefold()
            if folded in seen:
                raise ValueError(f"Duplicate archive entry: {name}")
            seen.add(folded)
            total += info.file_size
            if relative not in EXCLUDED:
                files.append((relative, info))
        if total > MAXIMUM_PAYLOAD:
            raise ValueError("The payload exceeds what one cabinet holds (2 GiB)")
        names = {relative for relative, _ in files}
        folders = {PurePosixPath(relative).parent.as_posix() for relative in names}
        clashes = sorted(name for name in names if name in folders)
        if clashes:
            raise ValueError(f"A file and a folder share a path: {clashes[0]}")
        check_release_contents(bundle, root, version, names)
    files.sort(key=lambda item: item[0])
    return Payload(archive, version, root, tuple(files), digest.hexdigest())


def check_release_contents(bundle, root, version, names):
    """The checks the ZIP itself passed: version, executable hash, README and license."""
    try:
        info = json.loads(bundle.read(f"{root}/BUILD-INFO.json"))
    except KeyError:
        raise ValueError("The archive has no BUILD-INFO.json") from None
    if info.get("version") != version or info.get("target") != TARGET or info.get("binary") != EXECUTABLE:
        raise ValueError(f"BUILD-INFO.json does not describe {EXECUTABLE} {version} for {TARGET}")
    for required in (EXECUTABLE, "README.txt", "LICENSE.txt", "midnight-elevated-listing.exe",
                     "THIRD-PARTY.txt"):
        if required not in names:
            raise ValueError(f"The archive has no {required}")
    executable = hashlib.sha256(bundle.read(f"{root}/{EXECUTABLE}")).hexdigest()
    if executable != str(info.get("binary_sha256", "")).lower():
        raise ValueError(f"{EXECUTABLE} does not match binary_sha256 in BUILD-INFO.json")
    readme = bundle.read(f"{root}/README.txt").decode("utf-8")
    source = f"{SOURCE_REPOSITORY}/tree/v{version}"
    if not readme.startswith(f"{PRODUCT} {version} ") or source not in readme:
        raise ValueError(f"README.txt must begin with '{PRODUCT} {version} ' and name {source}")
    if bundle.read(f"{root}/LICENSE.txt") != (ROOT / "LICENSE").read_bytes():
        raise ValueError("LICENSE.txt is not the repository's LICENSE")


def plan(payload):
    """Every table row of the installer database, from the payload alone."""
    version = payload.version
    folders = {"": []}
    for relative, info in payload.files:
        parent = PurePosixPath(relative).parent.as_posix()
        parent = "" if parent == "." else parent
        folders.setdefault(parent, []).append((relative, info))
        while parent:
            parent = PurePosixPath(parent).parent.as_posix()
            parent = "" if parent == "." else parent
            folders.setdefault(parent, [])
    directory_ids = {"": "INSTALLDIR"}
    for folder in folders:
        if folder:
            directory_ids[folder] = key("d", folder)
    taken = {folder: set() for folder in folders}
    # Per user, SetUserInstallFolder sets INSTALLDIR before CostFinalize.
    directories = [("TARGETDIR", None, "SourceDir"),
                   ("ProgramFiles64Folder", "TARGETDIR", "."),
                   ("ProgramMenuFolder", "TARGETDIR", "."),
                   ("INSTALLDIR", "ProgramFiles64Folder", msi_name(INSTALL_FOLDER, set()))]
    for folder in sorted(folders):
        if folder:
            path = PurePosixPath(folder)
            parent = "" if path.parent.as_posix() == "." else path.parent.as_posix()
            directories.append((directory_ids[folder], directory_ids[parent],
                                msi_name(path.name, taken[parent])))
    components, files, registry, remove_files, feature_components = [], [], [], [], []
    executable_key = None
    sequence = 0
    for folder in sorted(folders):
        component = "RootFolder" if not folder else key("c", folder)
        names = sorted(PurePosixPath(relative).name for relative, _ in folders[folder])
        component_code = code(f"component {folder}\n" + "\n".join(names))
        value = "r" + component[1:] if folder else "RootFolderKeyPath"
        registry.append((value, USER_OR_MACHINE, REGISTRY_KEY, component, "#1", component))
        components.append((component, component_code, directory_ids[folder],
                           COMPONENT_REGISTRY_KEY_PATH | COMPONENT_64BIT, None, value))
        feature_components.append(("Complete", component))
        remove_files.append(("x" + component[1:] if folder else "RemoveInstallFolder", component,
                             None, directory_ids[folder], REMOVE_ON_UNINSTALL))
        for relative, info in folders[folder]:
            sequence += 1
            file_key = "MainExecutable" if relative == EXECUTABLE else key("f", relative)
            if relative == EXECUTABLE:
                executable_key = file_key
            files.append((file_key, component, msi_name(PurePosixPath(relative).name, taken[folder]),
                          info.file_size, None, None, FILE_VITAL, sequence, relative))
    if executable_key is None:
        raise ValueError(f"The payload has no {EXECUTABLE}")
    registry.append(("RegistryKeyCleanup", USER_OR_MACHINE, REGISTRY_KEY, "*", None, "RootFolder"))
    components.append(("StartMenuShortcut", code("component start menu shortcut"),
                       "ProgramMenuFolder", COMPONENT_REGISTRY_KEY_PATH | COMPONENT_64BIT, None,
                       "StartMenuShortcutKeyPath"))
    registry.append(("StartMenuShortcutKeyPath", USER_OR_MACHINE, REGISTRY_KEY, "StartMenuShortcut", "#1",
                     "StartMenuShortcut"))
    feature_components.append(("Complete", "StartMenuShortcut"))
    shortcut_taken = set()
    shortcuts = [("StartMenuShortcut", "ProgramMenuFolder", msi_name(PRODUCT, shortcut_taken),
                  "StartMenuShortcut", f"[#{executable_key}]", None, "Dual-pane file manager",
                  None, None, None, None, "MESTARTFOLDER")]
    properties = [
        ("ProductCode", product_code(version)),
        ("ProductName", PRODUCT),
        ("ProductVersion", version),
        ("ProductLanguage", "1033"),
        ("Manufacturer", PUBLISHER),
        ("UpgradeCode", UPGRADE_CODE),
        ("ARPPRODUCTICON", "ProductIcon.ico"),
        ("ARPHELPLINK", SOURCE_REPOSITORY),
        ("ARPURLINFOABOUT", f"{SOURCE_REPOSITORY}/tree/v{version}"),
        ("ARPURLUPDATEINFO", f"{SOURCE_REPOSITORY}/releases"),
        ("ARPCOMMENTS", "Dual-pane file manager. Unsigned preview; see README.txt."),
        ("ARPNOMODIFY", "1"),
        # Passed to the elevated server: the copies that the UI sequence's AppSearch found
        # reach the execute sequence's launch conditions also after For all users.
        ("SecureCustomProperties", "ME_DOWNGRADE_DETECTED;ME_UPGRADE_DETECTED;ME_USER_COPY;ME_MACHINE_COPY"),
        # Dual-purpose package: per user unless MSIINSTALLPERUSER is cleared (the dialog's
        # "For all users", or MSIINSTALLPERUSER="" / ALLUSERS=1 on the command line).
        ("ALLUSERS", "2"),
        ("MSIINSTALLPERUSER", "1"),
        ("ME_SCOPE", "user"),
        ("ME_MAINTENANCE", "Repair"),
        ("DefaultUIFont", "METext"),
    ]
    upgrades = [
        (UPGRADE_CODE, version, None, None, UPGRADE_ONLY_DETECT, None, "ME_DOWNGRADE_DETECTED"),
        (UPGRADE_CODE, None, version, None, UPGRADE_MIGRATE_FEATURES, None, "ME_UPGRADE_DETECTED"),
    ]
    launch_conditions = [
        ("VersionNT64", f"{PRODUCT} requires 64-bit Windows 10 or later."),
        ("NOT ME_DOWNGRADE_DETECTED",
         f"A newer version of {PRODUCT} is already installed. Uninstall it first to install this one."),
        *((OTHER_SCOPE_CONDITIONS[scope], OTHER_SCOPE_REFUSALS[scope]) for scope in ("user", "machine")),
    ]
    return {
        "Property": properties,
        "Directory": directories,
        "Component": components,
        "Feature": [("Complete", None, PRODUCT, f"{PRODUCT} {version}", 1, 1, "INSTALLDIR", 0)],
        "FeatureComponents": feature_components,
        "File": files,
        "Media": [(1, sequence, None, "#" + CABINET, None, None)],
        "Registry": registry,
        "RemoveFile": remove_files,
        "Shortcut": shortcuts,
        "Upgrade": upgrades,
        "LaunchCondition": launch_conditions,
        "CustomAction": list(CUSTOM_ACTIONS),
        # The other scope's copy, by its registry key: the dialog disables that choice and
        # the launch conditions refuse it (OTHER_SCOPE_CONDITIONS).
        "AppSearch": [("ME_USER_COPY", "UserCopy"), ("ME_MACHINE_COPY", "MachineCopy")],
        "RegLocator": [("UserCopy", HKCU, REGISTRY_KEY, "RootFolder", REGISTRY_RAW_64BIT),
                       ("MachineCopy", HKLM, REGISTRY_KEY, "RootFolder", REGISTRY_RAW_64BIT)],
        **user_interface(),
        **{name: [(step[0], step[2] if len(step) > 2 else None, step[1]) for step in steps]
           for name, steps in SEQUENCES.items()},
    }


def _dialog(name, title_text, body, buttons, *, first=None, default=None, cancel=None,
            modal=True, width=WIDTH, height=HEIGHT, extra=()):
    """A dialog with a title line, body text, extra controls and buttons on the right."""
    controls = []
    if title_text:
        controls.append((name, "Title", "Text", 15, 10, width - 30, 22, TEXT, None,
                         "{\\METitle}" + title_text, None, None))
    if body:
        controls.append((name, "Body", "Text", 15, 38, width - 30, 40, TEXT, None, body, None, None))
    controls.extend(extra)
    controls.append((name, "BottomLine", "Line", 0, height - 36, width, 0, VISIBLE, None, None, None, None))
    x = width - 10
    for button, text, attributes, following in reversed(buttons):
        bw = 66
        x -= bw
        controls.append((name, button, "PushButton", x, height - 27, bw, 17, attributes, None, text,
                         following, None))
        x -= 6
    dialog = (name, 50, 50, width, height, DIALOG_MODAL if modal else DIALOG_MODELESS,
              "[ProductName] Setup", first or buttons[0][0], default, cancel)
    return dialog, controls


def user_interface():
    """The dialogs: scope choice (or repair/remove), progress, and the closing message."""
    dialogs, controls = [], []
    scope_extra = [
        ("ScopeDlg", "Scope", "RadioButtonGroup", 15, 80, WIDTH - 30, 40, CONTROL, "ME_SCOPE", None,
         "InstallUser", None),
        ("ScopeDlg", "UserText", "Text", 15, 124, WIDTH - 30, 30, TEXT, None,
         "Just for me: into %LOCALAPPDATA%\\Programs\\Midnight Explorer (MSI), without administrator "
         "rights.", None, None),
        ("ScopeDlg", "MachineText", "Text", 15, 156, WIDTH - 30, 30, TEXT, None,
         "For all users: into Program Files\\Midnight Explorer (MSI); Windows asks for administrator "
         "approval to install.", None, None),
        # Why the choice's Install button is disabled (one scope at a time).
        ("ScopeDlg", "UserCopyNote", "Text", 15, 192, WIDTH - 30, 36, HIDDEN_TEXT, None,
         OTHER_SCOPE_REFUSALS["machine"], None, None),
        ("ScopeDlg", "MachineCopyNote", "Text", 15, 192, WIDTH - 30, 36, HIDDEN_TEXT, None,
         OTHER_SCOPE_REFUSALS["user"], None, None),
    ]
    dialog, rows = _dialog(
        "ScopeDlg", "Install [ProductName] [ProductVersion]",
        "Choose who can use it on this computer. Both choices install the same files, and your "
        "settings stay in your profile either way.",
        [("InstallUser", "&Install", CONTROL, "InstallMachine"),
         ("InstallMachine", "&Install", ENABLED | ELEVATION_SHIELD, "Cancel"),
         ("Cancel", "Cancel", CONTROL, "Scope")],
        first="Scope", default="InstallUser", cancel="Cancel", extra=scope_extra)
    # The two Install buttons share one place; the scope shows one of them.
    machine_x = next(row[3] for row in rows if row[1] == "InstallMachine")
    rows = [row[:3] + (machine_x,) + row[4:] if row[1] == "InstallUser" else row for row in rows]
    dialogs.append(dialog)
    controls += rows
    maintenance_extra = [
        ("MaintenanceDlg", "Choice", "RadioButtonGroup", 15, 80, WIDTH - 30, 40, CONTROL,
         "ME_MAINTENANCE", None, "Continue", None),
    ]
    dialog, rows = _dialog(
        "MaintenanceDlg", "[ProductName] [ProductVersion] is installed",
        "Repair restores its files and Start menu entry. Remove uninstalls it; your settings stay.",
        [("Continue", "&Continue", CONTROL, "Cancel"), ("Cancel", "Cancel", CONTROL, "Choice")],
        first="Choice", default="Continue", cancel="Cancel", extra=maintenance_extra)
    dialogs.append(dialog)
    controls += rows
    progress_extra = [
        ("ProgressDlg", "ActionText", "Text", 15, 84, WIDTH - 30, 15, TEXT, None, None, None, None),
        ("ProgressDlg", "ProgressBar", "ProgressBar", 15, 104, WIDTH - 30, 12, VISIBLE | PROGRESS_95,
         None, "Progress", None, None),
    ]
    dialog, rows = _dialog(
        "ProgressDlg", "[ProductName] Setup", "Please wait while Setup makes the changes.",
        [("Cancel", "Cancel", CONTROL, None)], default="Cancel", cancel="Cancel", modal=False,
        extra=progress_extra)
    dialogs.append(dialog)
    controls += rows
    dialog, rows = _dialog(
        "CancelDlg", None, None, [("Yes", "&Yes", CONTROL, "No"), ("No", "&No", CONTROL, "Yes")],
        first="No", default="No", cancel="No", width=260, height=85,
        extra=[("CancelDlg", "Text", "Text", 15, 15, 230, 24, TEXT, None,
                "Cancel [ProductName] Setup? Nothing will be changed.", None, None)])
    dialogs.append(dialog)
    controls += rows
    for name, title, body in (
            ("ExitDlg", "[ProductName] Setup has finished",
             "Start [ProductName] from the Start menu."),
            ("UserExitDlg", "Setup was cancelled", "[ProductName] Setup was cancelled. Nothing was changed."),
            ("FatalErrorDlg", "Setup failed",
             "[ProductName] Setup ended because of an error, and nothing was changed. To record the "
             "reason, run msiexec /i <installer> /l*v <log file>.")):
        dialog, rows = _dialog(name, title, body, [("Finish", "&Finish", CONTROL, None)],
                               default="Finish", cancel="Finish")
        dialogs.append(dialog)
        controls += rows
    install = [
        ("[ALLUSERS]", "2", "1", 1),
        ("[MSIINSTALLPERUSER]", "1", 'ME_SCOPE = "user"', 2),
        ("[MSIINSTALLPERUSER]", "{}", 'ME_SCOPE = "machine"', 3),
        ("[INSTALLDIR]", USER_INSTALL_FOLDER, 'ME_SCOPE = "user"', 4),
        ("[INSTALLDIR]", MACHINE_INSTALL_FOLDER, 'ME_SCOPE = "machine"', 5),
        ("EndDialog", "Return", "1", 6),
    ]
    events = [("ScopeDlg", button, *event) for button in ("InstallUser", "InstallMachine") for event in install]
    events += [
        ("ScopeDlg", "Cancel", "EndDialog", "Exit", "1", 1),
        ("MaintenanceDlg", "Continue", "Reinstall", "ALL", 'ME_MAINTENANCE = "Repair"', 1),
        ("MaintenanceDlg", "Continue", "ReinstallMode", "ecmus", 'ME_MAINTENANCE = "Repair"', 2),
        ("MaintenanceDlg", "Continue", "Remove", "ALL", 'ME_MAINTENANCE = "Remove"', 3),
        ("MaintenanceDlg", "Continue", "EndDialog", "Return", "1", 4),
        ("MaintenanceDlg", "Cancel", "EndDialog", "Exit", "1", 1),
        ("ProgressDlg", "Cancel", "SpawnDialog", "CancelDlg", "1", 1),
        ("CancelDlg", "Yes", "EndDialog", "Exit", "1", 1),
        ("CancelDlg", "No", "EndDialog", "Return", "1", 1),
        ("ExitDlg", "Finish", "EndDialog", "Return", "1", 1),
        ("UserExitDlg", "Finish", "EndDialog", "Exit", "1", 1),
        ("FatalErrorDlg", "Finish", "EndDialog", "Exit", "1", 1),
    ]
    conditions = [
        ("ScopeDlg", "InstallUser", "Show", 'ME_SCOPE = "user"'),
        ("ScopeDlg", "InstallUser", "Hide", 'ME_SCOPE <> "user"'),
        ("ScopeDlg", "InstallUser", "Default", 'ME_SCOPE = "user"'),
        ("ScopeDlg", "InstallMachine", "Show", 'ME_SCOPE = "machine"'),
        ("ScopeDlg", "InstallMachine", "Hide", 'ME_SCOPE <> "machine"'),
        ("ScopeDlg", "InstallMachine", "Default", 'ME_SCOPE = "machine"'),
        # One scope at a time: a choice cannot install while the other scope has a copy.
        ("ScopeDlg", "InstallUser", "Disable", "ME_MACHINE_COPY"),
        ("ScopeDlg", "InstallUser", "Enable", "NOT ME_MACHINE_COPY"),
        ("ScopeDlg", "InstallMachine", "Disable", "ME_USER_COPY"),
        ("ScopeDlg", "InstallMachine", "Enable", "NOT ME_USER_COPY"),
        ("ScopeDlg", "UserCopyNote", "Show", 'ME_SCOPE = "machine" AND ME_USER_COPY'),
        ("ScopeDlg", "UserCopyNote", "Hide", 'ME_SCOPE <> "machine" OR NOT ME_USER_COPY'),
        ("ScopeDlg", "MachineCopyNote", "Show", 'ME_SCOPE = "user" AND ME_MACHINE_COPY'),
        ("ScopeDlg", "MachineCopyNote", "Hide", 'ME_SCOPE <> "user" OR NOT ME_MACHINE_COPY'),
    ]
    return {
        "TextStyle": [("METext", "Segoe UI", 9, None, 0), ("METitle", "Segoe UI", 12, None, 1)],
        "Dialog": dialogs,
        "Control": controls,
        "ControlEvent": events,
        "ControlCondition": conditions,
        "EventMapping": [("ProgressDlg", "ActionText", "ActionText", "Text"),
                         ("ProgressDlg", "ProgressBar", "SetProgress", "Progress")],
        "RadioButton": [
            ("ME_SCOPE", 1, "user", 0, 0, WIDTH - 30, 16, "&Just for me (no administrator rights)", None),
            ("ME_SCOPE", 2, "machine", 0, 22, WIDTH - 30, 16, "For &all users of this computer", None),
            ("ME_MAINTENANCE", 1, "Repair", 0, 0, WIDTH - 30, 16, "&Repair", None),
            ("ME_MAINTENANCE", 2, "Remove", 0, 22, WIDTH - 30, 16, "Re&move", None),
        ],
    }


def summary(payload):
    import msi_database as msi
    return {
        msi.PID_CODEPAGE: 1252,
        msi.PID_TITLE: "Installation Database",
        msi.PID_SUBJECT: f"{PRODUCT} {payload.version}",
        msi.PID_AUTHOR: PUBLISHER,
        msi.PID_KEYWORDS: "Installer",
        msi.PID_COMMENTS: f"Installs {PRODUCT} {payload.version} for the current user or all users.",
        msi.PID_TEMPLATE: "x64;1033",
        msi.PID_REVNUMBER: package_code(payload.version, payload.archive_sha256),
        msi.PID_PAGECOUNT: SCHEMA,
        msi.PID_WORDCOUNT: WORD_COUNT,
        msi.PID_APPNAME: f"{PRODUCT} scripts/package-msi.py",
        msi.PID_SECURITY: 2,
    }


def system_tool(name):
    """A tool from the Windows system folder, by full path."""
    import msi_database
    folder = Path(msi_database.system_folder())
    tool = folder / name
    if not tool.is_file():
        raise ValueError(f"{name} is not in {folder}")
    return tool


def extract_payload(payload, rows, folder):
    """Each payload file as `folder/<file key>`: short paths for makecab, whatever the depth."""
    folder.mkdir()
    with zipfile.ZipFile(payload.archive) as bundle:
        for row in rows:
            file_key, relative = row[0], row[8]
            with bundle.open(f"{payload.root}/{relative}") as source, open(folder / file_key, "xb") as target:
                shutil.copyfileobj(source, target, 1 << 20)


def write_cabinet(rows, source, work):
    """One LZX cabinet of the files in File.Sequence order, named by their File keys."""
    directives = work / "payload.ddf"
    lines = [".OPTION EXPLICIT", f".Set CabinetNameTemplate={CABINET}", f".Set DiskDirectoryTemplate={work}",
             f".Set SourceDir={source}", ".Set Cabinet=on", ".Set Compress=on",
             ".Set CompressionType=LZX", ".Set CompressionMemory=21", ".Set MaxDiskSize=0",
             ".Set CabinetFileCountThreshold=0", ".Set FolderFileCountThreshold=0",
             ".Set FolderSizeThreshold=0", ".Set MaxCabinetSize=0", ".Set UniqueFiles=on",
             ".Set InfFileName=NUL", ".Set RptFileName=NUL"]
    lines += [row[0] for row in sorted(rows, key=lambda row: row[7])]
    directives.write_text("\r\n".join(lines) + "\r\n", encoding="ascii")
    completed = subprocess.run([str(system_tool("makecab.exe")), "/F", str(directives)], cwd=work,
                               capture_output=True, text=True, errors="replace")
    cabinet = work / CABINET
    if completed.returncode != 0 or not cabinet.is_file():
        raise ValueError(f"makecab failed ({completed.returncode}): {completed.stdout}{completed.stderr}")
    return cabinet


def write_database(path, payload, tables, cabinet, icon):
    import msi_database as msi
    with msi.Database(path, create=True) as database:
        for name, columns in TABLES.items():
            database.execute(f"CREATE TABLE `{name}` ({columns})")
        for name in SEQUENCES:
            database.execute(f"CREATE TABLE `{name}` ({SEQUENCE_COLUMNS})")
        for name, rows in tables.items():
            width = 8 if name == "File" else None
            database.insert(name, [row[:width] for row in rows])
        database.insert("Icon", [("ProductIcon.ico", msi.Stream(icon))])
        database.execute("INSERT INTO `_Streams` (`Name`, `Data`) VALUES (?, ?)",
                         (CABINET, msi.Stream(cabinet)))
        database.set_summary(summary(payload))
        database.commit()


def default_output(archive):
    return Path(archive).absolute().parent


def build(archive, output=None, *, icon=ICON, repository=ROOT, out=sys.stdout):
    payload = read_payload(archive, expected_version=workspace_version(repository))
    output = package_release.build_output(output or default_output(archive), repository)
    output.mkdir(parents=True, exist_ok=True)
    msi_path = output / f"{payload.root}.msi"
    work = output / f".{payload.root}.msi-work"
    partial = output / f".{payload.root}.msi.partial"
    for existing in (msi_path, work, partial):
        if existing.exists():
            raise ValueError(f"Refused: {existing} exists")
    tables = plan(payload)
    work.mkdir()
    try:
        extract_payload(payload, tables["File"], work / "files")
        cabinet = write_cabinet(tables["File"], work / "files", work)
        write_database(partial, payload, tables, cabinet, icon)
        check(partial, payload, work / "check")
        rehearse(partial)
        os.rename(partial, msi_path)
    finally:
        shutil.rmtree(work, ignore_errors=True)
        if partial.exists():
            partial.unlink()
    print(msi_path, file=out)
    return msi_path


def installed_paths(directories, components, files):
    """File key -> path below INSTALLDIR, from the Directory, Component and File tables."""
    parents = {row[0]: (row[1], row[2]) for row in directories}

    def folder(identifier):
        parts = []
        while identifier != "INSTALLDIR":
            if identifier not in parents or len(parts) > 64:
                raise ValueError(f"Directory {identifier} is not below INSTALLDIR")
            parent, default = parents[identifier]
            parts.append(default.split("|")[-1].split(":")[0])
            identifier = parent
        return list(reversed(parts))

    component_folders = {row[0]: row[2] for row in components}
    return {row[0]: "/".join(folder(component_folders[row[1]]) + [row[2].split("|")[-1]])
            for row in files}


def check_scope_rules(tables):
    """The rules of the dual-purpose package, independent of `plan`; raises ValueError for
    the first problem. Per user by default (ALLUSERS=2, MSIINSTALLPERUSER=1) into the earlier
    per-user MSI's folder, per machine below Program Files; every key path and registry value
    follows the scope (root -1), nothing else is in a profile, one scope at a time (the other
    scope's copy refuses an installation), and nothing runs code: custom actions only set
    properties, and the refusals are launch conditions."""
    properties = dict(tables["Property"])
    if properties.get("ALLUSERS") != "2" or properties.get("MSIINSTALLPERUSER") != "1":
        raise ValueError("The package must be dual-purpose: ALLUSERS=2 and MSIINSTALLPERUSER=1")
    if properties.get("UpgradeCode") != UPGRADE_CODE:
        raise ValueError(f"The upgrade code must stay {UPGRADE_CODE}")
    if any(row[0] != UPGRADE_CODE for row in tables["Upgrade"]):
        raise ValueError("The package may only upgrade its own product")
    if any(row[1] != CUSTOM_ACTION_SET_PROPERTY for row in tables["CustomAction"]):
        raise ValueError("Custom actions may only set properties")
    actions = {row[0]: row for row in tables["CustomAction"]}
    if actions.get("SetUserInstallFolder", (None,) * 4)[2:] != ("INSTALLDIR", USER_INSTALL_FOLDER):
        raise ValueError("A per-user installation must go to the per-user MSI's folder")
    # The application finds its installation by the InstallLocation that this records
    # (docs/developer/updates.md#windows-installer-copies) and updates it with INSTALLDIR.
    execute = {row[0] for row in tables["InstallExecuteSequence"]}
    if (actions.get("SetInstallLocation", (None,) * 4)[2:] != ("ARPINSTALLLOCATION", "[INSTALLDIR]")
            or "SetInstallLocation" not in execute):
        raise ValueError("SetInstallLocation must record [INSTALLDIR] as the install location")
    directories = {row[0]: row for row in tables["Directory"]}
    if directories.get("INSTALLDIR", (None, None))[1] != "ProgramFiles64Folder":
        raise ValueError("INSTALLDIR must be below ProgramFiles64Folder")
    system_folders = {"TARGETDIR", "ProgramFiles64Folder", "ProgramMenuFolder"}
    if set(directories) & {"LocalAppDataFolder", "AppDataFolder", "PersonalFolder"}:
        raise ValueError("Only the per-user folder custom action may name a profile folder")
    registry = {row[0]: row for row in tables["Registry"]}
    if any(row[1] != USER_OR_MACHINE for row in tables["Registry"]):
        raise ValueError("Every registry value must follow the installation scope (root -1)")
    for component in tables["Component"]:
        path = registry.get(component[5])
        if (not component[3] & COMPONENT_REGISTRY_KEY_PATH or path is None
                or path[1] != USER_OR_MACHINE or path[5] != component[0]):
            raise ValueError(f"Component {component[0]} needs a key path that follows the scope")
    if any(row[1] != "ProgramMenuFolder" for row in tables["Shortcut"]):
        raise ValueError("Shortcuts belong in the Start menu's Programs folder")
    removed = {row[3] for row in tables["RemoveFile"] if row[2] is None}
    created = set(directories) - system_folders
    if created != removed:
        raise ValueError(f"Folders removed on uninstall must be exactly the package's own: "
                         f"{sorted(created ^ removed)}")
    install = {(row[2], row[3], row[4]) for row in tables["ControlEvent"]
               if row[0] == "ScopeDlg" and row[1] in ("InstallUser", "InstallMachine")}
    for event in (("[MSIINSTALLPERUSER]", "{}", 'ME_SCOPE = "machine"'),
                  ("[INSTALLDIR]", MACHINE_INSTALL_FOLDER, 'ME_SCOPE = "machine"'),
                  ("[INSTALLDIR]", USER_INSTALL_FOLDER, 'ME_SCOPE = "user"')):
        if event not in install:
            raise ValueError(f"The scope dialog must set {event[0]} for its choice")
    check_one_scope_rules(tables, actions)


def check_one_scope_rules(tables, actions):
    """One scope at a time: each scope's copy is found in its hive, the dialog disables the
    other choice's Install button, and the execute sequence (only) refuses the other scope."""
    searches = {row[0]: row[1] for row in tables["AppSearch"]}
    locators = {row[0]: tuple(row[1:]) for row in tables["RegLocator"]}
    for name, root in (("ME_USER_COPY", HKCU), ("ME_MACHINE_COPY", HKLM)):
        if locators.get(searches.get(name)) != (root, REGISTRY_KEY, "RootFolder", REGISTRY_RAW_64BIT):
            raise ValueError(f"{name} must find the RootFolder value of an installed copy")
    disabled = {(row[1], row[3]) for row in tables["ControlCondition"]
                if row[0] == "ScopeDlg" and row[2] == "Disable"}
    for button, copy in (("InstallUser", "ME_MACHINE_COPY"), ("InstallMachine", "ME_USER_COPY")):
        if (button, copy) not in disabled:
            raise ValueError(f"The scope dialog must disable {button} while {copy} is set")
    launch = dict(tables["LaunchCondition"])
    for scope, condition in OTHER_SCOPE_CONDITIONS.items():
        if launch.get(condition) != OTHER_SCOPE_REFUSALS[scope]:
            raise ValueError(f"A {scope} installation must be refused while the other scope has a copy")
    # The UI sequence evaluates the launch conditions before ScopeDlg, with the default scope:
    # the rule applies only where SetScopeRule sets it, after AppSearch in the execute sequence.
    execute = {row[0]: row[2] for row in tables["InstallExecuteSequence"] if row[1] is None}
    ui = {row[0] for row in tables["InstallUISequence"]}
    order = [execute.get(name) for name in ("AppSearch", "SetScopeRule", "LaunchConditions")]
    if (actions.get("SetScopeRule", (None,) * 4)[2:] != (SCOPE_RULE, "1") or "SetScopeRule" in ui
            or None in order or not order[0] < order[1] < order[2]):
        raise ValueError("The scope rule must apply in the execute sequence only, after AppSearch")


def check(msi_path, payload, work):
    """Checks an MSI against its release payload; raises ValueError for the first problem."""
    import msi_database as msi
    expected = plan(payload)
    with msi.Database(msi_path) as database:
        info = database.summary()
        wanted = summary(payload)
        for pid in (msi.PID_TEMPLATE, msi.PID_REVNUMBER, msi.PID_WORDCOUNT, msi.PID_PAGECOUNT,
                    msi.PID_AUTHOR, msi.PID_SUBJECT):
            if info.get(pid) != wanted[pid]:
                raise ValueError(f"Summary property {pid} is {info.get(pid)!r}, expected {wanted[pid]!r}")
        tables = {row[0] for row in database.rows("SELECT `Name` FROM `_Tables`")}
        unexpected = tables - set(TABLES) - set(SEQUENCES)
        if unexpected:
            raise ValueError(f"Unexpected tables (nothing may run code): {sorted(unexpected)}")
        actual = {name: sorted(database.rows(f"SELECT * FROM `{name}`"), key=repr)
                  for name in expected if name in tables}
        for name, rows in expected.items():
            rows = sorted((row[:8] if name == "File" else tuple(row) for row in rows), key=repr)
            if actual.get(name) != rows:
                raise ValueError(f"Table {name} differs from the release payload")
        check_scope_rules(actual)
        if database.rows("SELECT `Name` FROM `Icon`") != [("ProductIcon.ico",)]:
            raise ValueError("The product icon is missing")
        paths = installed_paths(actual["Directory"], actual["Component"], actual["File"])
        wanted_paths = {row[0]: row[8] for row in expected["File"]}
        if paths != wanted_paths:
            raise ValueError("The installed file paths differ from the release payload")
        work.mkdir()
        cabinet = work / CABINET
        database.save_stream(f"SELECT `Data` FROM `_Streams` WHERE `Name` = '{CABINET}'", cabinet)
    check_cabinet(cabinet, payload, wanted_paths, work / "expanded")


def check_cabinet(cabinet, payload, paths, folder):
    """Every file in the embedded cabinet equals its ZIP entry, and nothing else is in it."""
    folder.mkdir()
    completed = subprocess.run([str(system_tool("expand.exe")), str(cabinet), "-F:*", str(folder)],
                               capture_output=True, text=True, errors="replace")
    if completed.returncode != 0:
        raise ValueError(f"expand failed ({completed.returncode}): {completed.stdout}{completed.stderr}")
    present = {entry.name for entry in folder.iterdir()}
    if present != set(paths):
        raise ValueError("The cabinet does not hold exactly the payload's files")
    with zipfile.ZipFile(payload.archive) as bundle:
        for file_key, relative in paths.items():
            expected = hashlib.sha256()
            with bundle.open(f"{payload.root}/{relative}") as source:
                for chunk in iter(lambda: source.read(1 << 20), b""):
                    expected.update(chunk)
            with open(folder / file_key, "rb") as stream:
                if hashlib.file_digest(stream, "sha256").hexdigest() != expected.hexdigest():
                    raise ValueError(f"{relative} in the cabinet differs from the release archive")


def verify(msi_path, archive, *, repository=ROOT, out=sys.stdout):
    payload = read_payload(archive)
    msi_path = Path(msi_path).absolute()
    if msi_path.name != f"{payload.root}.msi":
        raise ValueError(f"The installer of {payload.root} must be named {payload.root}.msi")
    work = package_release.build_output(repository / "_build" / f".{payload.root}.msi-check", repository)
    if work.exists():
        raise ValueError(f"Refused: {work} exists")
    work.parent.mkdir(parents=True, exist_ok=True)
    try:
        check(msi_path, payload, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    rehearse(msi_path)
    print(f"{msi_path.name}: {len(payload.files)} files match {Path(archive).name}", file=out)


def ui_actions():
    """The actions the UI sequence runs before its first dialog, in order (no dialogs)."""
    steps = sorted((step for step in SEQUENCES["InstallUISequence"] if step[1] > 0), key=lambda s: s[1])
    # Dialogs and the hand-off to the server need a real installation, and so does
    # MigrateFeatureStates (it reports 1626 in a session without one).
    dialogs = {"ScopeDlg", "MaintenanceDlg", "ProgressDlg", "ExecuteAction", "MigrateFeatureStates"}
    return [step[0] for step in steps if step[0] not in dialogs]


def rehearse(msi_path):
    """Run the UI sequence's actions for both scopes in a session that installs nothing,
    then the launch conditions that keep one scope at a time.

    A table an action needs but the package lacks (error 2228) or a failing search stops
    a real installation before its first dialog; this finds it at build time.
    """
    import msi_database as msi
    for scope in ((("MSIINSTALLPERUSER", "1"),), (("ALLUSERS", "1"), ("MSIINSTALLPERUSER", ""))):
        try:
            msi.rehearse_actions(msi_path, ui_actions(), scope)
        except msi.MsiError as error:
            raise ValueError(f"{Path(msi_path).name} cannot start an installation: {error}") from error
    rehearse_scope_rule(msi_path)


# How each scope is chosen: (scope, properties). Per user: the package's default and Just for
# me, and an empty ALLUSERS; per machine: ALLUSERS=1 on the command line (the package's
# MSIINSTALLPERUSER=1 stays) and For all users.
SCOPE_CHOICES = (
    ("user", (("ALLUSERS", "2"), ("MSIINSTALLPERUSER", "1"))),
    ("user", (("ALLUSERS", ""), ("MSIINSTALLPERUSER", ""))),
    ("machine", (("ALLUSERS", "1"), ("MSIINSTALLPERUSER", "1"))),
    ("machine", (("ALLUSERS", "2"), ("MSIINSTALLPERUSER", ""))),
)
COPIES = {"user": "ME_USER_COPY", "machine": "ME_MACHINE_COPY"}


def scope_rule_cases():
    """(scope, properties, copies found, execute sequence, maintenance, refused) for every
    choice of scope and copies: refused only in the execute sequence of a new installation
    while the other scope has a copy."""
    cases = []
    for scope, properties in SCOPE_CHOICES:
        other = "machine" if scope == "user" else "user"
        for copies in ((), ("user",), ("machine",), ("user", "machine")):
            for execute in (False, True):
                for maintenance in (False, True):
                    refused = execute and not maintenance and other in copies
                    cases.append((scope, properties, copies, execute, maintenance, refused))
    return cases


def rehearse_scope_rule(msi_path):
    """The launch conditions of one scope at a time, evaluated by Windows Installer itself in
    a session that installs nothing (the copies found are set, not searched): the execute
    sequence refuses a scope while the other scope has a copy, but neither an upgrade in
    the same scope nor maintenance, and the UI sequence, which evaluates them before
    ScopeDlg with the default scope, refuses nothing."""
    import msi_database as msi
    name = Path(msi_path).name
    with msi.Session(msi_path) as session:
        for scope, properties, copies, execute, maintenance, refused in scope_rule_cases():
            for property_name, value in properties:
                session.set(property_name, value)
            for property_name in (*COPIES.values(), SCOPE_RULE, "Installed"):
                session.set(property_name, "")
            for copy in copies:
                session.set(COPIES[copy], "#1")
            if maintenance:
                session.set("Installed", "00:00:00")
            if execute and session.do("SetScopeRule") != 0:
                raise ValueError(f"{name}: SetScopeRule failed")
            session.messages.clear()
            result = session.do("LaunchConditions")
            case = (f"{scope} installation ({dict(properties)}) with copies {list(copies) or 'none'} in the "
                    f"{'execute' if execute else 'UI'} sequence{' (maintenance)' if maintenance else ''}")
            if refused and (result == 0 or OTHER_SCOPE_REFUSALS[scope] not in " ".join(session.messages)):
                raise ValueError(f"{name}: a {case} is not refused ({result}: {session.messages})")
            if not refused and result != 0:
                raise ValueError(f"{name}: a {case} is refused ({result}: {session.messages})")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    make = commands.add_parser("build", help="build the MSI beside (or below --output) the release ZIP")
    make.add_argument("--zip", required=True, help="midnight-explorer-<version>-x86_64-pc-windows-msvc.zip")
    make.add_argument("--output", help="output folder below the checkout's _build (default: the ZIP's)")
    checked = commands.add_parser("verify", help="check an MSI against its release ZIP")
    checked.add_argument("--msi", required=True)
    checked.add_argument("--zip", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "build":
            build(args.zip, args.output)
        else:
            verify(args.msi, args.zip)
    except (ValueError, OSError) as error:
        raise SystemExit(f"Refused: {error}") from None
    return 0


if __name__ == "__main__":
    sys.exit(main())
