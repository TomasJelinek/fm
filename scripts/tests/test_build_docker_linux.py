"""Docker build delegation uses literal argv and never builds during a dry run."""
import csv
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux Docker build helper")
class LinuxDockerBuildTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="docker helper ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name) / "checkout with spaces & dollar$"
        (self.root / "scripts").mkdir(parents=True)
        for relative in ("scripts/build-docker.sh", "scripts/docker-install-deps.sh",
                         "rust-toolchain.toml"):
            shutil.copyfile(ROOT / relative, self.root / relative)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "commands.jsonl"
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + "/usr/bin:/bin",
                                DOCKER_HELPER_LOG=str(self.log), DOCKER_IMAGE_PRESENT="1")
        for name in ("FM_DOCKER_DNS", "CARGO_HOME", "RUSTUP_HOME", "CARGO_TARGET_DIR",
                     "DOCKER_CREATE_EXIT", "DOCKER_START_EXIT", "DOCKER_INSPECT_EXIT",
                     "DOCKER_COMMIT_EXIT", "DOCKER_RUN_EXIT", "DOCKER_SETUP_STATUS"):
            self.environment.pop(name, None)
        self.command("id", 'case "$1" in -u) printf "1234\\n";; -g) printf "4321\\n";; '
                     '*) exit 98;; esac\n')
        fixture = '''#!/usr/bin/python3
import json
import os
from pathlib import Path
import sys

tool = Path(sys.argv[0]).name
arguments = sys.argv[1:]
with open(os.environ["DOCKER_HELPER_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps({"tool": tool, "arguments": arguments}) + "\\n")
if tool != "docker":
    sys.exit(99)
if arguments[:2] == ["image", "inspect"]:
    sys.exit(0 if os.environ["DOCKER_IMAGE_PRESENT"] == "1" else 1)
if arguments[0] == "create":
    status = int(os.environ.get("DOCKER_CREATE_EXIT", "0"))
    if status == 0:
        print("fixture-container")
    sys.exit(status)
if arguments[0] == "inspect":
    print(os.environ.get("DOCKER_SETUP_STATUS", "0"))
if arguments[0] in ("start", "inspect", "commit", "run"):
    sys.exit(int(os.environ.get("DOCKER_" + arguments[0].upper() + "_EXIT", "0")))
if arguments[0] == "rm":
    sys.exit(0)
sys.exit(98)
'''
        for name in ("docker", "cargo", "rustup", "curl", "sudo", "apt-get", "dnf", "zypper"):
            path = self.bin / name
            path.write_text(fixture, encoding="utf-8")
            path.chmod(0o755)

    def command(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\nset -eu\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def run_helper(self, *arguments):
        return subprocess.run(["/bin/sh", str(self.root / "scripts/build-docker.sh"), *arguments],
                              cwd=self.root.parent, env=self.environment, text=True,
                              capture_output=True, timeout=20)

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text(encoding="utf-8").splitlines()]

    def docker_calls(self):
        calls = self.calls()
        self.assertTrue(all(call["tool"] == "docker" for call in calls), calls)
        return [call["arguments"] for call in calls]

    def assert_no_output_created(self):
        self.assertFalse((self.root / "_build").exists())
        self.assertFalse((self.root / ".tools").exists())

    def test_cached_image_runs_full_entrypoint_as_invoking_user_with_writable_caches(self):
        for profile in ("debug", "release"):
            with self.subTest(profile=profile):
                if self.log.exists():
                    self.log.unlink()
                result = self.run_helper(profile)
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = self.docker_calls()
                self.assertEqual(len(calls), 2, calls)
                self.assertEqual(calls[0][:2], ["image", "inspect"])
                image = calls[0][2]
                self.assertRegex(image, r"^midnight-explorer-build:[0-9a-f]{64}$")
                self.assertEqual(calls[1], [
                    "run", "--rm", "--user", "1234:4321",
                    "--mount", f'type=bind,"src={self.root}",dst=/workspace,readonly',
                    "--mount", f'type=bind,"src={self.root}/_build",dst=/workspace/_build',
                    "--mount", f'type=bind,"src={self.root}/.tools",dst=/workspace/.tools',
                    "--workdir", "/workspace",
                    "--env", "FM_BUILD_BACKEND=native",
                    "--env", "HOME=/workspace/.tools/docker/home",
                    "--env", "CARGO_HOME=/workspace/.tools/docker/cargo",
                    "--env", "RUSTUP_HOME=/opt/rustup",
                    "--env", "PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    "--env", "CARGO_TARGET_DIR=/workspace/_build",
                    "--env", "TMPDIR=/workspace/_build/tmp",
                    image, "sh", "/workspace/build.sh", profile,
                ])
                for relative in ("_build/tmp", ".tools/docker/cargo", ".tools/docker/home"):
                    self.assertTrue((self.root / relative).is_dir(), relative)
                self.assertFalse((self.root / "_build/docker").exists())

    def test_default_profile_is_debug(self):
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.docker_calls()[-1][-3:], ["sh", "/workspace/build.sh", "debug"])

    def test_setup_and_build_mounts_preserve_commas_and_quotes_in_source_paths(self):
        checkout = self.root.with_name('checkout, with "double quotes"')
        self.root.rename(checkout)
        self.root = checkout
        self.bin = self.root / "bin"
        self.log = self.root / "commands.jsonl"
        self.environment.update(PATH=str(self.bin) + os.pathsep + "/usr/bin:/bin",
                                DOCKER_HELPER_LOG=str(self.log), DOCKER_IMAGE_PRESENT="0")
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.docker_calls()
        for operation, expected in (
            ("create", [["type=bind", f"src={self.root}", "dst=/workspace", "readonly"]]),
            ("run", [
                ["type=bind", f"src={self.root}", "dst=/workspace", "readonly"],
                ["type=bind", f"src={self.root}/_build", "dst=/workspace/_build"],
                ["type=bind", f"src={self.root}/.tools", "dst=/workspace/.tools"],
            ]),
        ):
            with self.subTest(operation=operation):
                arguments = next(call for call in calls if call[0] == operation)
                mounts = [next(csv.reader([arguments[index + 1]], strict=True))
                          for index, argument in enumerate(arguments) if argument == "--mount"]
                self.assertEqual(mounts, expected)

    def test_dns_is_literal_for_image_setup_and_build(self):
        marker = self.root / "unexpected shell evaluation"
        dns = f"10.0.0.53;$(touch '{marker}')"
        self.environment.update(DOCKER_IMAGE_PRESENT="0", FM_DOCKER_DNS=dns)
        result = self.run_helper("release")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.docker_calls()
        self.assertEqual([call[0] for call in calls],
                         ["image", "create", "start", "inspect", "commit", "rm", "run"])
        setup = calls[1]
        self.assertEqual(setup[:3], ["create", "--dns", dns])
        self.assertEqual(setup[3:], [
            "--mount", f'type=bind,"src={self.root}",dst=/workspace,readonly',
            "--env", "RUSTUP_HOME=/opt/rustup", "--env", "CARGO_HOME=/opt/cargo",
            "--env", "PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "ubuntu:24.04", "sh", "/workspace/scripts/docker-install-deps.sh",
        ])
        self.assertEqual(calls[2], ["start", "--attach", "fixture-container"])
        self.assertEqual(calls[3], ["inspect", "--format", "{{.State.ExitCode}}", "fixture-container"])
        self.assertEqual(calls[4], ["commit", "fixture-container", calls[0][2]])
        self.assertEqual(calls[5], ["rm", "fixture-container"])
        run = calls[-1]
        self.assertEqual(run[run.index("--dns"):run.index("--dns") + 2], ["--dns", dns])
        self.assertEqual(run[-4:], [calls[0][2], "sh", "/workspace/build.sh", "release"])
        self.assertFalse(marker.exists())

    def test_recipe_and_toolchain_changes_invalidate_image_cache(self):
        images = []
        for relative in (None, "scripts/docker-install-deps.sh", "rust-toolchain.toml"):
            if relative:
                with (self.root / relative).open("a", encoding="utf-8") as recipe:
                    recipe.write("\n# changed cached input\n")
            if self.log.exists():
                self.log.unlink()
            result = self.run_helper()
            self.assertEqual(result.returncode, 0, result.stderr)
            images.append(self.docker_calls()[0][2])
        self.assertEqual(len(set(images)), 3, images)

    def test_dry_run_plans_setup_and_full_build_without_any_actions(self):
        self.command("mkdir", 'printf "mkdir\\n" >> "$DOCKER_HELPER_LOG"\nexit 99\n')
        self.environment.update(DOCKER_IMAGE_PRESENT="0", FM_DOCKER_DNS="10.0.0.53")
        for arguments in (("release", "--dry-run"), ("--dry-run", "debug")):
            with self.subTest(arguments=arguments):
                result = self.run_helper(*arguments)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("FM_BUILD_BACKEND=native", result.stdout)
                self.assertIn("[sh] [/workspace/build.sh]", result.stdout)
                self.assertIn("[sh] [/workspace/scripts/docker-install-deps.sh]", result.stdout)
                self.assertEqual(result.stdout.count("[--dns] [10.0.0.53]"), 2)
                self.assertIn("dst=/workspace/.tools", result.stdout)
                self.assertIn("dst=/workspace,readonly", result.stdout)
                self.assertFalse(self.log.exists())
                self.assert_no_output_created()

    def test_invalid_arguments_stop_before_tools_or_directories(self):
        for arguments in (("clean",), ("release", "debug"), ("debug", "extra"),
                          ("release", "--dry-run", "--unknown"), ("",)):
            with self.subTest(arguments=arguments):
                result = self.run_helper(*arguments)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(self.log.exists())
                self.assert_no_output_created()

    def test_missing_docker_stops_before_creating_output(self):
        (self.bin / "docker").unlink()
        for name in ("dirname", "cat", "sha256sum"):
            (self.bin / name).symlink_to(shutil.which(name))
        self.environment["PATH"] = str(self.bin)
        result = self.run_helper()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("Docker", result.stderr)
        self.assertFalse(self.log.exists())
        self.assert_no_output_created()

    def test_failed_setup_status_cleans_container_and_skips_commit_and_build(self):
        self.environment.update(DOCKER_IMAGE_PRESENT="0", DOCKER_SETUP_STATUS="37")
        result = self.run_helper()
        self.assertEqual(result.returncode, 37, result.stderr)
        calls = self.docker_calls()
        self.assertEqual([call[0] for call in calls], ["image", "create", "start", "inspect", "rm"])
        self.assertEqual(calls[-1], ["rm", "-f", "fixture-container"])

    def test_malformed_setup_status_cleans_container_without_publishing_image(self):
        self.environment.update(DOCKER_IMAGE_PRESENT="0", DOCKER_SETUP_STATUS="not-an-exit-code")
        result = self.run_helper()
        self.assertNotEqual(result.returncode, 0, result.stderr)
        calls = self.docker_calls()
        self.assertEqual([call[0] for call in calls], ["image", "create", "start", "inspect", "rm"])
        self.assertEqual(calls[-1], ["rm", "-f", "fixture-container"])

    def test_create_failure_never_starts_or_removes_an_unowned_container(self):
        self.environment.update(DOCKER_IMAGE_PRESENT="0", DOCKER_CREATE_EXIT="43")
        result = self.run_helper()
        self.assertEqual(result.returncode, 43, result.stderr)
        self.assertEqual([call[0] for call in self.docker_calls()], ["image", "create"])

    def test_setup_command_failures_cleanup_and_stop_before_build(self):
        for variable, status, operations in (
            ("DOCKER_START_EXIT", "47", ["image", "create", "start", "rm"]),
            ("DOCKER_INSPECT_EXIT", "49", ["image", "create", "start", "inspect", "rm"]),
            ("DOCKER_COMMIT_EXIT", "51", ["image", "create", "start", "inspect", "commit", "rm"]),
        ):
            with self.subTest(operation=variable):
                if self.log.exists():
                    self.log.unlink()
                self.environment.update(DOCKER_IMAGE_PRESENT="0", **{variable: status})
                result = self.run_helper()
                self.environment.pop(variable)
                self.assertEqual(result.returncode, int(status), result.stderr)
                calls = self.docker_calls()
                self.assertEqual([call[0] for call in calls], operations)
                self.assertEqual(calls[-1], ["rm", "-f", "fixture-container"])

    def test_build_failure_status_reaches_caller(self):
        self.environment["DOCKER_RUN_EXIT"] = "53"
        result = self.run_helper()
        self.assertEqual(result.returncode, 53, result.stderr)
        self.assertEqual([call[0] for call in self.docker_calls()], ["image", "run"])


if __name__ == "__main__":
    unittest.main()
