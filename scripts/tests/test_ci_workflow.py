"""The CI workflow keeps its coverage opt-ins, explicit toolchain and bounded jobs.

A plain-text check (the standard library has no YAML parser) of invariants whose
silent loss once removed coverage: archive tests became opt-in for normal builds
and the Linux CI job stopped opting in. Where PyYAML is installed, the workflow and
the local actions are also parsed and their structure checked.
"""
import importlib.util
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"


def jobs(text):
    body = text.split("\njobs:\n", 1)[1]
    names = list(re.finditer(r"^  ([a-z0-9-]+):\n", body, re.MULTILINE))
    return {match.group(1): body[match.end():names[index + 1].start() if index + 1 < len(names) else None]
            for index, match in enumerate(names)}


def step(job, name):
    """The text of the job's step with this exact name."""
    steps = [text for text in re.split(r"\n      - ", job) if re.match(rf"(?:if: .*\n\s*)?name: {re.escape(name)}\n", text)]
    if len(steps) != 1:
        raise AssertionError(f"Expected one step named {name!r}, found {len(steps)}")
    return steps[0]


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.text = WORKFLOW.read_text(encoding="utf-8")
        self.jobs = jobs(self.text)

    def test_archive_job_opts_into_plugin_tests_and_a_release_package_on_both_platforms(self):
        job = self.jobs["archive-filesystem"]
        self.assertRegex(job, r'scripts/build\.ps1" .*-Profile release -Test')
        self.assertRegex(job, r'bash "\$plugin/scripts/build\.sh" \$sdk \$build \$package release --test')

    def test_every_job_is_bounded_and_installs_the_pinned_toolchain_explicitly(self):
        self.assertNotIn("rustup show", self.text)
        for name, job in self.jobs.items():
            with self.subTest(job=name):
                self.assertRegex(job, r"\n    timeout-minutes: \d+\n")
                command = re.search(r"^\s*(?:run: )?cargo ", job, re.MULTILINE)
                if command:
                    self.assertIn("uses: ./.github/actions/rust-setup", job)
                    self.assertLess(job.index("rust-setup"), command.start())
                if "sparse-checkout:" in job and "./.github/actions/" in job:
                    self.assertIn("/.github/actions/", job.split("sparse-checkout:", 1)[1].split("- uses:", 1)[0])

    def test_rust_setup_reads_the_toolchain_file(self):
        action = (ROOT / ".github/actions/rust-setup/action.yml").read_text(encoding="utf-8")
        self.assertIn("rustup toolchain install $channel --profile $toolchainProfile @components", action)
        self.assertIn("rust-toolchain.toml", action)

    def test_superseded_runs_are_cancelled_and_pull_requests_run_once(self):
        self.assertIn("\nconcurrency:\n", self.text)
        self.assertIn("cancel-in-progress:", self.text)
        on = self.text.split("\non:\n", 1)[1].split("\n\n", 1)[0]
        self.assertRegex(on, r"push:\n    branches: \[main\]")
        self.assertIn("pull_request:", on)

    def test_post_gate_steps_reuse_the_workspace_feature_selection(self):
        check = self.jobs["check"]
        after_gates = check.split("ci-workspace-tests.py --retire-successful-test-artifacts", 1)[1]
        self.assertNotRegex(after_gates, r"cargo test -p ")
        self.assertNotIn("--include-ignored", check)

    def test_image_thumbnailer_has_independent_ci_with_strict_checks(self):
        job = self.jobs["thumbnail-image"]
        self.assertIn("needs: public-worker-sdk", job)
        self.assertIn('"$plugin/scripts/ci.ps1"', job)
        self.assertIn('bash "$plugin/scripts/ci.sh"', job)
        for script in ("ci.ps1", "ci.sh"):
            text = (ROOT / "plugins/midnight-thumbnail-image/scripts" / script).read_text(encoding="utf-8")
            with self.subTest(script=script):
                for step in ("image-decoder/native", "fmt", "test", "clippy", "-D", "warnings", "build", "--release"):
                    self.assertIn(step, text)

    def test_public_protocol_workspaces_and_conformance_have_ci(self):
        sdk = self.jobs["public-worker-sdk"]
        for workspace in ("worker-api", "thumbnail-api", "filesystem-api", "image-viewer-api", "web-viewer-api"):
            self.assertIn(workspace, sdk)
        self.assertIn("api/scripts/check-abi.sh", self.jobs["public-sdk"])
        self.assertIn("tests/test-linux.sh", self.jobs["viewer-conformance"])
        self.assertIn("tests/test-windows.ps1", self.jobs["viewer-conformance"])

    def test_isolated_x11_tests_run_on_a_display_that_never_resets(self):
        # Without -noreset, Xvfb resets when a test's last connection closes and drops
        # the next test's first connection; the drag/drop tests connect per test.
        check = self.jobs["check"]
        for name, opt_in in (("drag and drop", "MIDNIGHT_DND_TEST_ISOLATED=1"),
                             ("clipboard", "MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1")):
            with self.subTest(step=name):
                text = step(check, f"Native X11 {name} (isolated display)")
                self.assertIn("if: runner.os == 'Linux'", text)
                self.assertIn('xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset"', text)
                self.assertIn(opt_in, text)
                self.assertIn("--ignored --test-threads=1", text)
        clipboard = step(check, "Native X11 clipboard (isolated display)")
        self.assertIn("env -u WAYLAND_DISPLAY", clipboard)
        self.assertIn("clipboard::tests::x11_ file_clipboard::x11::tests::", clipboard)

    def test_installed_archive_worker_qualification_uses_the_exported_fixtures(self):
        archive = self.jobs["archive-filesystem"]
        self.assertIn('--test worker_fixtures --locked --offline -- --ignored', archive)
        self.assertIn("name: archive-fixtures-${{ runner.os }}", archive)
        check = self.jobs["check"]
        self.assertIn("name: archive-fixtures-${{ runner.os }}", check)
        text = step(check, "Installed archive worker streaming, all formats, cooperative cancellation and package discovery")
        for variable in ("ARCHIVE_CAPTURE_INSTALLATION", "MIDNIGHT_ARCHIVE_FIXTURES", "MIDNIGHT_ARCHIVE_TEST_ZIP"):
            self.assertIn(f"$env:{variable} = ", text)
        self.assertIn("scripts/archive-test-zip.py", text)
        for selection in ("test:streaming_worker --ignored", "lib captured_source_tests:: --ignored",
                          "test:discovery_kinds --ignored"):
            self.assertIn(f"run midnight-filesystem-host {selection}", text)

    @unittest.skipUnless(importlib.util.find_spec("yaml"), "PyYAML is not installed")
    def test_workflow_and_local_actions_parse_with_a_consistent_structure(self):
        import yaml
        workflow = yaml.safe_load(self.text)
        parsed = workflow["jobs"]
        self.assertEqual(set(parsed), set(self.jobs))
        for name, job in parsed.items():
            with self.subTest(job=name):
                self.assertIsInstance(job["timeout-minutes"], int)
                needs = job.get("needs", [])
                self.assertLessEqual(set([needs] if isinstance(needs, str) else needs), set(parsed))
                for entry in job["steps"]:
                    self.assertEqual(len({"uses", "run"} & set(entry)), 1, entry)
                    if entry.get("uses", "").startswith("./"):
                        self.assertTrue((ROOT / entry["uses"] / "action.yml").is_file(), entry["uses"])
        actions = sorted((ROOT / ".github/actions").glob("*/action.yml"))
        self.assertTrue(actions)
        for action in actions:
            with self.subTest(action=action.parent.name):
                definition = yaml.safe_load(action.read_text(encoding="utf-8"))
                self.assertEqual(definition["runs"]["using"], "composite")
                for entry in definition["runs"]["steps"]:
                    self.assertEqual(len({"uses", "run"} & set(entry)), 1, entry)
                    if "run" in entry:
                        self.assertIn("shell", entry, "Composite run steps need an explicit shell")


if __name__ == "__main__":
    unittest.main()
