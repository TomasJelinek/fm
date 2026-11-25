#!/usr/bin/env python3
"""Author a per-user MSI database and cabinet inputs using only the standard library.

Windows Installer COM and makecab.exe consume the generated inputs; this module
does not install products or execute anything from the source package.
"""
import argparse
import hashlib
import importlib.util
import json
import re
import shutil
import uuid
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("release_packaging", Path(__file__).with_name("package-release.py"))
packaging = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packaging)
NAMESPACE = uuid.UUID("719fbdb4-f0eb-43df-aa63-d75a6f52dd8e")


def guid(name):
    return "{" + str(uuid.uuid5(NAMESPACE, name)).upper() + "}"


def identifier(prefix, name):
    return prefix + hashlib.sha256(name.casefold().encode("utf-8")).hexdigest()[:32]


def author(source, version, work):
    """Freeze every payload byte and emit IDT tables plus a makecab directive file."""
    source = packaging.real_directory(source, "MSI source")
    work = Path(work).absolute()
    root = Path(__file__).resolve().parents[1]
    packaging.build_output(work, root)
    if source == work or work.is_relative_to(source) or source.is_relative_to(work):
        raise ValueError("MSI work and source directories must be separate")
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
        raise ValueError("MSI requires a numeric major.minor.build version")
    parts = [int(part) for part in version.split(".")]
    if parts[0] > 255 or parts[1] > 255 or parts[2] > 65535:
        raise ValueError("MSI version exceeds Windows Installer limits")
    files = packaging.plain_tree(source, maximum=30000)
    if source / "me.exe" not in files or source / "BUILD-INFO.json" not in files:
        raise ValueError("MSI requires a complete application package")
    metadata = json.loads((source / "BUILD-INFO.json").read_text(encoding="utf-8-sig"))
    if metadata.get("version") != version or metadata.get("target") != "x86_64-pc-windows-msvc":
        raise ValueError("MSI source version or target mismatch")
    seen = set()
    total = 0
    for file in files:
        name = file.relative_to(source).as_posix()
        for part in file.relative_to(source).parts:
            packaging.portable_component(part)
            try:
                part.encode("cp1252")
            except UnicodeEncodeError as error:
                raise ValueError("MSI payload names must fit its Windows-1252 string pool: " + name) from error
        if name.casefold() in seen:
            raise ValueError("Case-colliding MSI payload paths")
        seen.add(name.casefold())
        size = file.stat().st_size
        total += size
        if size > 2_000_000_000 or total > 2_000_000_000:
            raise ValueError("MSI embedded cabinet exceeds the supported 2 GB limit")
    work.mkdir(parents=True, exist_ok=False)
    (work / "cab-input").mkdir()
    (work / "tables").mkdir()
    tables = {}

    def table(name, columns, types, keys, rows):
        tables[name] = {"columns": columns.split(), "rows": rows}
        lines = [columns.split(), types.split(), [name, *keys.split()], *rows]
        encoded = "\r\n".join("\t".join("" if item is None else str(item) for item in row)
                                 for row in lines) + "\r\n"
        (work / "tables" / (name + ".idt")).write_bytes(encoded.encode("cp1252"))

    upgrade = guid("upgrade/windows/x64/user")
    product = guid("product/windows/x64/user/" + version)
    properties = {
        "ProductCode": product, "UpgradeCode": upgrade, "ProductName": "Midnight Explorer",
        "ProductVersion": version, "ProductLanguage": "1033", "Manufacturer": "Midnight Explorer",
        "MSIINSTALLPERUSER": "1", "ARPNOMODIFY": "1",
        "ARPCOMMENTS": "File manager", "DefaultUIFont": "DefaultFont",
        "SecureCustomProperties": "INSTALLFOLDER;OLDPRODUCTS;NEWERPRODUCTS",
    }
    table("Property", "Property Value", "s72 l0", "Property", [list(p) for p in properties.items()])
    directories = [["TARGETDIR", None, "SourceDir"], ["LocalAppDataFolder", "TARGETDIR", "."],
                   ["ProgramsFolder", "LocalAppDataFolder", "Programs"],
                   ["INSTALLFOLDER", "ProgramsFolder", "MIDNIG~1|Midnight Explorer"],
                   ["ProgramMenuFolder", "TARGETDIR", "."]]
    all_directories = sorted({parent for file in files for parent in file.relative_to(source).parents
                              if str(parent) != "."}, key=lambda p: (len(p.parts), p.as_posix()))
    for directory in all_directories:
        relative = directory.as_posix()
        parent = "INSTALLFOLDER" if str(directory.parent) == "." else identifier("D", directory.parent.as_posix())
        directories.append([identifier("D", relative), parent, identifier("X", relative)[:8] + "|" + directory.name])
    table("Directory", "Directory Directory_Parent DefaultDir", "s72 S72 l255", "Directory", directories)
    payload, components, file_rows, registry, feature_components = [], [], [], [], []
    for sequence, file in enumerate(files, 1):
        relative = file.relative_to(source)
        name = relative.as_posix()
        file_id, component, registry_id = (identifier(prefix, name) for prefix in ("F", "C", "R"))
        directory = "INSTALLFOLDER" if str(relative.parent) == "." else identifier("D", relative.parent.as_posix())
        destination = work / "cab-input" / file_id
        with packaging.plain_file(file) as original, destination.open("xb") as output:
            shutil.copyfileobj(original, output)
        size = destination.stat().st_size
        digest = packaging.sha256_file(destination)
        payload.append({"id": file_id, "path": name, "bytes": size, "sha256": digest})
        components.append([component, guid("component/windows/x64/user/" + name.casefold()),
                           directory, 260, None, registry_id])
        file_rows.append([file_id, component, "P" + f"{sequence:07X}" + "|" + relative.name,
                          size, None, None, 512, sequence])
        registry.append([registry_id, 1, "Software\\Midnight Explorer\\Installer\\Components",
                         component, "[#" + file_id + "]", component])
        feature_components.append(["Application", component])
    main_component = identifier("C", "me.exe")
    registry.append(["InstallLocation", 1, "Software\\Midnight Explorer\\Installer", "InstallLocation",
                     "[INSTALLFOLDER]", main_component])
    table("Component", "Component ComponentId Directory_ Attributes Condition KeyPath",
          "s72 S38 s72 i2 S255 S72", "Component", components)
    table("File", "File Component_ FileName FileSize Version Language Attributes Sequence",
          "s72 s72 l255 i4 S72 S20 I2 i4", "File", file_rows)
    table("Registry", "Registry Root Key Name Value Component_", "s72 i2 l255 L255 L0 s72", "Registry", registry)
    table("Feature", "Feature Feature_Parent Title Description Display Level Directory_ Attributes",
          "s38 S38 L64 L255 I2 i2 S72 i2", "Feature",
          [["Application", None, "Midnight Explorer", None, 0, 1, "INSTALLFOLDER", 0]])
    table("FeatureComponents", "Feature_ Component_", "s38 s72", "Feature_ Component_", feature_components)
    table("Media", "DiskId LastSequence DiskPrompt Cabinet VolumeLabel Source", "i2 i4 L64 S255 S32 S72", "DiskId",
          [[1, len(files), None, "#payload.cab", None, None]])
    table("Shortcut", "Shortcut Directory_ Name Component_ Target Arguments Description Hotkey Icon_ IconIndex ShowCmd WkDir",
          "s72 s72 l128 s72 s72 S255 L255 I2 S72 I2 I2 S72", "Shortcut",
          [["StartMenu", "ProgramMenuFolder", "MIDNIG~1|Midnight Explorer", main_component,
            "[#" + identifier("F", "me.exe") + "]", None, "Midnight Explorer", None, None, None, 1, "INSTALLFOLDER"]])
    table("AppSearch", "Property Signature_", "s72 s72", "Property Signature_", [["INSTALLFOLDER", "PriorLocation"]])
    # AppSearch requires this table even for raw registry lookups whose signature
    # has no file-search row. Omitting it makes the native action fail with 2228.
    table("Signature", "Signature FileName MinVersion MaxVersion MinSize MaxSize MinDate MaxDate Languages",
          "s72 S255 S20 S20 I4 I4 I4 I4 S255", "Signature", [])
    table("RegLocator", "Signature_ Root Key Name Type", "s72 i2 s255 S255 I2", "Signature_",
          [["PriorLocation", 1, "Software\\Midnight Explorer\\Installer", "InstallLocation", 18]])
    table("Upgrade", "UpgradeCode VersionMin VersionMax Language Attributes Remove ActionProperty",
          "s38 S20 S20 S255 i4 S255 s72", "UpgradeCode VersionMin VersionMax Language Attributes",
          [[upgrade, None, version, None, 1, None, "OLDPRODUCTS"],
           [upgrade, version, None, None, 258, None, "NEWERPRODUCTS"]])
    table("LaunchCondition", "Condition Description", "s255 l255", "Condition",
          [["NOT ALLUSERS AND NOT MSIINSTALLPERMACHINE", "This installer supports the current user only."],
           ["VersionNT64", "Midnight Explorer requires 64-bit Windows."],
           ["Installed OR NOT NEWERPRODUCTS", "A newer version of Midnight Explorer is already installed."]])
    actions = [("FindRelatedProducts", 25), ("AppSearch", 50), ("LaunchConditions", 100),
               ("ValidateProductID", 700), ("CostInitialize", 800), ("FileCost", 900), ("CostFinalize", 1000)]
    execute = actions + [("MigrateFeatureStates", 1200), ("InstallValidate", 1400), ("InstallInitialize", 1500),
                         ("RemoveExistingProducts", 1510), ("ProcessComponents", 1600), ("UnpublishFeatures", 1800),
                         ("RemoveRegistryValues", 2600), ("RemoveShortcuts", 3200), ("RemoveFiles", 3500),
                         ("InstallFiles", 4000), ("CreateShortcuts", 4500), ("WriteRegistryValues", 5000),
                         ("RegisterUser", 6000), ("RegisterProduct", 6100), ("PublishFeatures", 6300),
                         ("PublishProduct", 6400), ("InstallFinalize", 6600)]
    table("InstallExecuteSequence", "Action Condition Sequence", "s72 S255 I2", "Action",
          [[name, "OLDPRODUCTS" if name == "RemoveExistingProducts" else None, order] for name, order in execute])
    table("InstallUISequence", "Action Condition Sequence", "s72 S255 I2", "Action",
          [[name, None, order] for name, order in actions] +
          [["InstallFolderDlg", "NOT Installed", 1100], ["ExecuteAction", None, 1300]])
    table("TextStyle", "TextStyle FaceName Size Color StyleBits", "s72 s32 i2 I4 I2", "TextStyle",
          [["DefaultFont", "Tahoma", 9, None, 0]])
    table("Dialog", "Dialog HCentering VCentering Width Height Attributes Title Control_First Control_Default Control_Cancel",
          "s72 i2 i2 i2 i2 i4 L128 s50 S50 S50", "Dialog",
          [["InstallFolderDlg", 50, 50, 370, 145, 3, "Midnight Explorer Setup", "Folder", "Install", "Cancel"]])
    table("Control", "Dialog_ Control Type X Y Width Height Attributes Property Text Control_Next Help",
          "s72 s50 s20 i2 i2 i2 i2 I4 S72 L0 S50 L50", "Dialog_ Control",
          [["InstallFolderDlg", "Description", "Text", 15, 15, 340, 30, 3, None,
            "Install Midnight Explorer for your Windows account. Choose its folder:", None, None],
           ["InstallFolderDlg", "Folder", "PathEdit", 15, 55, 340, 20, 3, "INSTALLFOLDER", None, "Install", None],
           ["InstallFolderDlg", "Install", "PushButton", 195, 105, 75, 23, 3, None, "Install", "Cancel", None],
           ["InstallFolderDlg", "Cancel", "PushButton", 280, 105, 75, 23, 3, None, "Cancel", "Folder", None]])
    table("ControlEvent", "Dialog_ Control_ Event Argument Condition Ordering",
          "s72 s50 s50 l255 S255 I2", "Dialog_ Control_ Event Argument Condition",
          [["InstallFolderDlg", "Install", "SetTargetPath", "INSTALLFOLDER", "1", 1],
           ["InstallFolderDlg", "Install", "EndDialog", "Return", "1", 2],
           ["InstallFolderDlg", "Cancel", "EndDialog", "Exit", "1", 1]])
    # Force the database string-pool codepage before importing any authored strings.
    (work / "tables/_ForceCodepage.idt").write_bytes(b"\r\n\r\n1252\t_ForceCodepage\r\n")
    ddf = [".OPTION EXPLICIT", ".Set CabinetNameTemplate=payload.cab", ".Set DiskDirectoryTemplate=cabinet",
           ".Set CompressionType=LZX", ".Set CompressionMemory=21", ".Set Cabinet=ON", ".Set Compress=ON",
           ".Set MaxDiskSize=0", ".Set MaxCabinetSize=0", ".Set FolderSizeThreshold=0"]
    ddf.extend('"cab-input/' + entry["id"] + '" "' + entry["id"] + '"' for entry in payload)
    (work / "payload.ddf").write_text("\r\n".join(ddf) + "\r\n", encoding="ascii", newline="")
    plan = {"version": version, "product_code": product, "upgrade_code": upgrade,
            "package_code": "{" + str(uuid.uuid4()).upper() + "}", "work": str(work),
            "payload": payload, "tables": tables}
    (work / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return plan


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    author(args.source, args.version, args.work)
