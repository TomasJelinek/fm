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
# %LOCALAPPDATA%\Programs\<INSTALL_FOLDER>. Not "Midnight Explorer": that folder is the
# destination of the application's own "Move to my profile..." copy, which an installer
# must neither overwrite nor have replaced under it.
INSTALL_FOLDER = "Midnight Explorer (MSI)"
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

# Word count: compressed files (2) and no elevated privileges needed (8).
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
}
SEQUENCE_COLUMNS = ("`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT "
                    "PRIMARY KEY `Action`")
# Standard action order (the numbers Windows Installer's documentation suggests). The
# only custom actions only set properties (type 51); nothing runs code.
SEQUENCES = {
    "InstallUISequence": (
        ("FindRelatedProducts", 25), ("LaunchConditions", 100), ("ValidateProductID", 700),
        ("CostInitialize", 800), ("FileCost", 900), ("CostFinalize", 1000),
        ("MigrateFeatureStates", 1200), ("ExecuteAction", 1300)),
    "InstallExecuteSequence": (
        ("FindRelatedProducts", 25), ("LaunchConditions", 100), ("ValidateProductID", 700),
        ("CostInitialize", 800), ("FileCost", 900), ("CostFinalize", 1000),
        ("SetInstallLocation", 1001), ("SetShortcutWorkingFolder", 1002),
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
# Add/Remove Programs shows the install folder; the Start menu shortcut starts in the
# user's profile folder, as the Desktop shortcut of scripts/windows-shortcut.ps1 does.
CUSTOM_ACTIONS = (
    ("SetInstallLocation", CUSTOM_ACTION_SET_PROPERTY, "ARPINSTALLLOCATION", "[INSTALLDIR]"),
    ("SetShortcutWorkingFolder", CUSTOM_ACTION_SET_PROPERTY, "MEUSERPROFILE", "[%USERPROFILE]"),
)
HKCU = 1
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
    directories = [("TARGETDIR", None, "SourceDir"),
                   ("LocalAppDataFolder", "TARGETDIR", "."),
                   ("ProgramMenuFolder", "TARGETDIR", "."),
                   ("UserProgramsFolder", "LocalAppDataFolder", msi_name("Programs", set())),
                   ("INSTALLDIR", "UserProgramsFolder", msi_name(INSTALL_FOLDER, set()))]
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
        registry.append((value, HKCU, REGISTRY_KEY, component, "#1", component))
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
    # The installer's own folders below the profile: removed on uninstall when empty.
    registry.append(("RegistryKeyCleanup", HKCU, REGISTRY_KEY, "*", None, "RootFolder"))
    remove_files.append(("RemoveProgramsFolder", "RootFolder", None, "UserProgramsFolder",
                         REMOVE_ON_UNINSTALL))
    components.append(("StartMenuShortcut", code("component start menu shortcut"),
                       "ProgramMenuFolder", COMPONENT_REGISTRY_KEY_PATH | COMPONENT_64BIT, None,
                       "StartMenuShortcutKeyPath"))
    registry.append(("StartMenuShortcutKeyPath", HKCU, REGISTRY_KEY, "StartMenuShortcut", "#1",
                     "StartMenuShortcut"))
    feature_components.append(("Complete", "StartMenuShortcut"))
    shortcut_taken = set()
    shortcuts = [("StartMenuShortcut", "ProgramMenuFolder", msi_name(PRODUCT, shortcut_taken),
                  "StartMenuShortcut", f"[#{executable_key}]", None, "Dual-pane file manager",
                  None, None, None, None, "MEUSERPROFILE")]
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
        ("SecureCustomProperties", "ME_DOWNGRADE_DETECTED;ME_UPGRADE_DETECTED"),
    ]
    upgrades = [
        (UPGRADE_CODE, version, None, None, UPGRADE_ONLY_DETECT, None, "ME_DOWNGRADE_DETECTED"),
        (UPGRADE_CODE, None, version, None, UPGRADE_MIGRATE_FEATURES, None, "ME_UPGRADE_DETECTED"),
    ]
    launch_conditions = [
        ("VersionNT64", f"{PRODUCT} requires 64-bit Windows 10 or later."),
        ("NOT ME_DOWNGRADE_DETECTED",
         f"A newer version of {PRODUCT} is already installed. Uninstall it first to install this one."),
        ("NOT ALLUSERS", f"This package installs {PRODUCT} for the current user only; do not set ALLUSERS."),
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
        **{name: [(action, None, number) for action, number in actions]
           for name, actions in SEQUENCES.items()},
    }


def summary(payload):
    import msi_database as msi
    return {
        msi.PID_CODEPAGE: 1252,
        msi.PID_TITLE: "Installation Database",
        msi.PID_SUBJECT: f"{PRODUCT} {payload.version}",
        msi.PID_AUTHOR: PUBLISHER,
        msi.PID_KEYWORDS: "Installer",
        msi.PID_COMMENTS: f"Installs {PRODUCT} {payload.version} for the current user.",
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
        properties = dict(actual["Property"])
        if "ALLUSERS" in properties or "MSIINSTALLPERUSER" in properties:
            raise ValueError("The package must install per user without ALLUSERS")
        if any(row[1] != CUSTOM_ACTION_SET_PROPERTY for row in actual["CustomAction"]):
            raise ValueError("Custom actions may only set properties")
        if database.rows("SELECT `Name` FROM `Icon`") != [("ProductIcon.ico",)]:
            raise ValueError("The product icon is missing")
        components = actual["Component"]
        registry = {row[0]: row for row in actual["Registry"]}
        for component in components:
            path = registry.get(component[5])
            if (not component[3] & COMPONENT_REGISTRY_KEY_PATH or path is None
                    or path[1] != HKCU or path[5] != component[0]):
                raise ValueError(f"Component {component[0]} needs an HKCU key path")
        removed = {row[3] for row in actual["RemoveFile"] if row[2] is None}
        created = {row[0] for row in actual["Directory"]} - {"TARGETDIR", "LocalAppDataFolder",
                                                              "ProgramMenuFolder"}
        if created - removed:
            raise ValueError(f"Folders not removed on uninstall: {sorted(created - removed)}")
        paths = installed_paths(actual["Directory"], components, actual["File"])
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
    print(f"{msi_path.name}: {len(payload.files)} files match {Path(archive).name}", file=out)


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
