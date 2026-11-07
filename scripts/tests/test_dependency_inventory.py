"""Regression checks for the documentation inventory, using synthetic Cargo graphs."""
import importlib.util
from pathlib import Path
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

    def test_versions_remain_distinct_and_targets_are_combined(self):
        root = package("app", source=None)
        first, second = package("lib"), package("lib", "2.0.0")
        metadata = graph(root, [first, second], {"app": [first, second]})
        rows = self.module.inventory({("app", target): metadata
                                      for target in ("Windows", "Linux")})
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["targets"], ["Linux", "Windows"])
        self.assertEqual([row["version"] for row in rows], ["1.0.0", "2.0.0"])

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


if __name__ == "__main__":
    unittest.main()
