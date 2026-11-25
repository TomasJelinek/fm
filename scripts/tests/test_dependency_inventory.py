"""Regression checks for the documentation inventory, using synthetic Cargo graphs."""
import csv
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "dependency-inventory.py"


def package(name, version="1.0.0", license="MIT", source="registry+crates.io"):
    return {"id": f"{source}#{name}@{version}", "name": name, "version": version,
            "license": license, "source": source}


def graph(root, dependencies, edges, unused=()):
    packages = [root, *dependencies, *unused]
    return {"workspace_members": [root["id"]], "packages": packages,
            "resolve": {"nodes": [
                {"id": item["id"], "deps": [{"pkg": child["id"]} for child in
                 edges.get(item["name"], [])]} for item in packages]}}


class InventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("dependency_inventory", SCRIPT)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def test_only_reachable_packages_and_direct_relationships_are_reported(self):
        root = package("app", source=None)
        direct, indirect, unused = map(package, ("direct", "indirect", "unused"))
        metadata = graph(root, [direct, indirect],
                         {"app": [direct], "direct": [indirect]}, [unused])
        rows = self.module.inventory({("app", "Windows"): metadata})
        self.assertEqual([row["name"] for row in rows], ["direct", "indirect"])
        self.assertEqual(rows[0]["direct_by"], ["app"])
        self.assertEqual(rows[1]["direct_by"], [])

    def test_vendored_test_workspace_remains_third_party(self):
        app = package("app", source=None)
        app["metadata"] = None
        vendor = package("renderer", source=None)
        vendor["metadata"] = {"midnight": {"vendored-third-party": True}}
        test_dep = package("gpu-test")
        rows = self.module.inventory({
            ("app", "Windows"): graph(app, [vendor], {"app": [vendor]}),
            ("renderer-tests", "Windows"): graph(vendor, [test_dep],
                                                {"renderer": [test_dep]})})
        by_name = {row["name"]: row for row in rows}
        self.assertEqual(set(by_name), {"renderer", "gpu-test"})
        self.assertEqual(by_name["renderer"]["direct_by"], ["app"])
        self.assertEqual(by_name["renderer"]["scopes"], ["app", "renderer-tests"])
        self.assertEqual(by_name["gpu-test"]["direct_by"], [])

    def test_versions_remain_distinct_and_targets_are_combined(self):
        root = package("app", source=None)
        first, second = package("lib"), package("lib", "2.0.0")
        metadata = graph(root, [first, second], {"app": [first, second]})
        rows = self.module.inventory({("app", target): metadata
                                      for target in ("Windows", "Linux")})
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["targets"], ["Linux", "Windows"])
        self.assertEqual([row["version"] for row in rows], ["1.0.0", "2.0.0"])

    def test_cli_reads_both_darwin_captures_only_for_native_macos(self):
        temporary = SCRIPT.parent.parent / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary) as directory:
            captures = Path(directory) / "captures"
            captures.mkdir()
            output = Path(directory) / "inventory"

            def capture(scope, target, dependencies):
                root = package(f"fixture-{scope}", source=None)
                metadata = graph(root, dependencies, {root["name"]: dependencies})
                path = captures / f"{scope}-{target}.json"
                path.write_text(json.dumps(metadata), encoding="utf-8")
                path.with_suffix(".tree").write_text(
                    "".join(f'{item["name"]} v{item["version"]}\n'
                            for item in [root, *dependencies]), encoding="utf-8")

            for scope in self.module.SCOPES:
                if scope == "native-macos":
                    continue
                dependencies = [package("app-only")] if scope == "app" else []
                for target in ("x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"):
                    capture(scope, target, dependencies)
            for target in ("x86_64-apple-darwin", "aarch64-apple-darwin"):
                capture("native-macos", target, [package("libc", "0.2.191")])

            command = [sys.executable, "-B", str(SCRIPT), "--metadata-dir", str(captures),
                       "--output-dir", str(output)]
            completed = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            with (output / "cargo-licenses.csv").open(encoding="utf-8", newline="") as stream:
                rows = {row["name"]: row for row in csv.DictReader(stream)}
            self.assertEqual(set(rows), {"app-only", "libc"})
            self.assertEqual(rows["libc"]["scopes"], "native-macos")
            self.assertEqual(rows["libc"]["targets"], "macOS arm64; macOS x86-64")
            self.assertEqual(rows["libc"]["direct_by"], "fixture-native-macos")
            self.assertEqual(rows["app-only"]["targets"], "Linux; Windows")
            checked = subprocess.run([*command, "--check"], capture_output=True,
                                     text=True, check=False)
            self.assertEqual(checked.returncode, 0, checked.stderr)

    def test_cargo_tree_excludes_metadata_feature_union_only_dependencies(self):
        root = package("app", source=None)
        active, unused = package("active"), package("optional-renderer")
        metadata = graph(root, [active, unused], {"app": [active, unused]})
        capture = ("app", "Linux")
        rows = self.module.inventory({capture: metadata},
                                     {capture: {("app", "1.0.0"), ("active", "1.0.0")}})
        self.assertEqual([row["name"] for row in rows], ["active"])

    def test_released_public_sdk_is_project_code_not_third_party(self):
        sdk = package("our-sdk", source=None)
        released = package("our-sdk", source="registry+our-api")
        plugin = package("our-plugin", source=None)
        external = package("external")
        rows = self.module.inventory({
            ("api", "Linux"): graph(sdk, [external], {"our-sdk": [external]}),
            ("basic", "Linux"): graph(plugin, [released, external],
                                       {"our-plugin": [released], "our-sdk": [external]})})
        self.assertEqual([row["name"] for row in rows], ["external"])
        self.assertEqual(rows[0]["scopes"], ["api", "basic"])

    def test_historical_public_sdk_from_exact_project_registry_keeps_external_dependencies(self):
        sdk = package("our-sdk", "0.4.0", source=None)
        released = package("our-sdk", "0.1.0", license=None,
                           source="registry+https://midnight-explorer.invalid/api-index")
        plugin = package("our-plugin", source=None)
        external = package("external")
        rows = self.module.inventory({
            ("api", "Linux"): graph(sdk, [external], {"our-sdk": [external]}),
            ("basic", "Linux"): graph(plugin, [released, external],
                                       {"our-plugin": [released], "our-sdk": [external]})})
        self.assertEqual([row["name"] for row in rows], ["external"])
        self.assertEqual(rows[0]["scopes"], ["api", "basic"])

    def test_historical_sdk_requires_exact_registry_and_nonvendored_member_name(self):
        registry = "registry+https://midnight-explorer.invalid/api-index"
        cases = [
            ("registry+https://github.com/rust-lang/crates.io-index", "our-sdk", False),
            ("registry+https://mirror.example/api-index", "our-sdk", False),
            (registry + "/mirror", "our-sdk", False),
            (registry, "our-sdk-lookalike", False),
            (registry, "our-sdk", True),
        ]
        for source, name, vendored in cases:
            with self.subTest(source=source, name=name, vendored=vendored):
                sdk = package("our-sdk", "0.4.0", source=None)
                if vendored:
                    sdk["metadata"] = {"midnight": {"vendored-third-party": True}}
                released = package(name, "0.1.0", license=None, source=source)
                plugin = package("our-plugin", source=None)
                rows = self.module.inventory({
                    ("api", "Linux"): graph(sdk, [], {}),
                    ("basic", "Linux"): graph(plugin, [released],
                                               {"our-plugin": [released]})})
                historical = next(row for row in rows
                                  if row["name"] == name and row["version"] == "0.1.0")
                self.assertEqual(historical["license"], "UNKNOWN")

    def test_missing_or_conflicting_license_is_never_silently_approved(self):
        root = package("app", source=None)
        missing = package("lib", license=None)
        metadata = graph(root, [missing], {"app": [missing]})
        rows = self.module.inventory({("app", "Linux"): metadata})
        self.assertEqual(rows[0]["license"], "UNKNOWN")
        changed = package("lib", license="MIT")
        with self.assertRaises(ValueError):
            self.module.inventory({("app", "Linux"): metadata,
                                   ("app", "Windows"): graph(root, [changed], {"app": [changed]})})

    def test_incomplete_resolve_graph_fails_instead_of_omitting_dependencies(self):
        root, dep = package("app", source=None), package("missing")
        metadata = graph(root, [], {"app": [dep]})
        with self.assertRaises(ValueError):
            self.module.inventory({("app", "Linux"): metadata})

    def test_tree_package_unreachable_in_metadata_is_not_silently_omitted(self):
        root, disconnected = package("app", source=None), package("disconnected")
        metadata = graph(root, [], {}, [disconnected])
        capture = ("app", "Linux")
        selected = {("app", "1.0.0"), ("disconnected", "1.0.0")}
        with self.assertRaisesRegex(ValueError, "Unreachable Cargo tree packages"):
            self.module.inventory({capture: metadata}, {capture: selected})

    def test_empty_or_partial_tree_cannot_drop_workspace_members(self):
        root = package("app", source=None)
        capture = ("app", "Linux")
        with self.assertRaisesRegex(ValueError, "Cargo tree omits workspace members"):
            self.module.inventory({capture: graph(root, [], {})}, {capture: set()})

    def test_capture_script_captures_every_scope_as_documented(self):
        root = SCRIPT.parent.parent
        script = (SCRIPT.parent / "capture-dependency-inventory.ps1").read_text(encoding="utf-8")
        scopes = {name: (manifest, rest) for name, manifest, rest in
                  re.findall(r"@\{ Name = '([^']+)'; Manifest = '([^']+)'([^}]*)\}", script)}
        self.assertEqual({name: manifest for name, (manifest, _) in scopes.items()}, self.module.SCOPES)

        def listed(variable):
            return re.findall(r"'([^']+)'", re.search(rf"\${variable} = @\(([^)]*)\)", script).group(1))
        self.assertEqual(listed("hostTargets"), list(self.module.TARGETS))
        self.assertEqual(listed("darwinTargets"), list(self.module.SCOPE_TARGETS["native-macos"]))
        # A workspace that locks a released project SDK needs that SDK's configuration.
        for name, (manifest, rest) in scopes.items():
            with self.subTest(scope=name):
                lock = (root / manifest).with_name("Cargo.lock").read_text(encoding="utf-8")
                pinned = f'source = "{self.module.PROJECT_REGISTRY_SOURCE}"' in lock
                self.assertEqual("Sdk = " in rest, pinned)
        # The features of docs/THIRD_PARTY.md, and no others.
        documentation = (root / "docs/THIRD_PARTY.md").read_text(encoding="utf-8")
        options = {name: " ".join(re.findall(r"'([^']+)'", re.search(r"Options = @\(([^)]*)\)", rest).group(1)))
                   for name, (_, rest) in scopes.items() if "Options = " in rest}
        self.assertEqual(options, {"renderer-tests": "--no-default-features --features wgpu",
                                   "thumbnail-video": "--features system,playback-worker"})
        for name, flags in options.items():
            self.assertIn(f"For `{name}`, append `{flags}`", documentation)


if __name__ == "__main__":
    unittest.main()
