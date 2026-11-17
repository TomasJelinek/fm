"""The CI workflow keeps its coverage opt-ins, explicit toolchain and bounded jobs.

A plain-text check (the standard library has no YAML parser) of invariants whose
silent loss once removed coverage: archive tests became opt-in for normal builds
and the Linux CI job stopped opting in.
"""
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

    def test_public_protocol_workspaces_and_conformance_have_ci(self):
        sdk = self.jobs["public-worker-sdk"]
        for workspace in ("worker-api", "thumbnail-api", "filesystem-api", "image-viewer-api", "web-viewer-api"):
            self.assertIn(workspace, sdk)
        self.assertIn("api/scripts/check-abi.sh", self.jobs["public-sdk"])
        self.assertIn("tests/test-linux.sh", self.jobs["viewer-conformance"])
        self.assertIn("tests/test-windows.ps1", self.jobs["viewer-conformance"])


if __name__ == "__main__":
    unittest.main()
