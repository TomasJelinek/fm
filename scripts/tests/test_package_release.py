"""Regression checks for the release dependency inventory."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parents[1] / "package-release.py")
packaging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packaging)


def package(name, source="registry+https://github.com/rust-lang/crates.io-index"):
    return {"name": name, "version": "0.1.0", "source": source}


class DependencyInventoryTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {"packages": [package("midnight-explorer", None),
                                      package("runtime"), package("derive"),
                                      package("unused-workspace-renderer")]}

    def test_uses_application_selection_and_preserves_proc_macro_notices(self):
        tree = ("midnight-explorer v0.1.0 (/checkout/app)\n"
                "runtime v0.1.0\n"
                "derive v0.1.0 (proc-macro)\n"
                "runtime v0.1.0 (*)\n")
        result = packaging.dependency_packages(self.metadata, tree)
        self.assertEqual([entry["name"] for entry in result], ["derive", "runtime"])

    def test_indented_tree_cannot_silently_omit_dependency_notices(self):
        tree = "midnight-explorer v0.1.0\n\u2514\u2500\u2500 runtime v0.1.0\n"
        with self.assertRaises(ValueError):
            packaging.dependency_packages(self.metadata, tree)

    def test_missing_or_ambiguous_metadata_is_rejected(self):
        tree = "midnight-explorer v0.1.0\nruntime v0.1.0\n"
        with self.assertRaises(ValueError):
            packaging.dependency_packages(self.metadata, tree + "missing v0.1.0\n")
        self.metadata["packages"].append(package("runtime", "git+https://example.invalid/fork"))
        with self.assertRaises(ValueError):
            packaging.dependency_packages(self.metadata, tree)


if __name__ == "__main__":
    unittest.main()
