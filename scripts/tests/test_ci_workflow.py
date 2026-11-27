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
INVENTORY_WORKFLOW = ROOT / ".github/workflows/dependency-inventory.yml"
# The rules that are not about ci.yml's own jobs hold for every workflow.
WORKFLOWS = sorted((ROOT / ".github/workflows").glob("*.yml"))


def jobs(text):
    body = text.split("\njobs:\n", 1)[1]
    names = list(re.finditer(r"^  ([a-z0-9-]+):\n", body, re.MULTILINE))
    return {match.group(1): body[match.end():names[index + 1].start() if index + 1 < len(names) else None]
            for index, match in enumerate(names)}


def action(name):
    """The text of a local composite action."""
    return (ROOT / ".github/actions" / name / "action.yml").read_text(encoding="utf-8")


def uses(job, reference):
    """The text of the job's one step that uses this action."""
    steps = [text for text in re.split(r"\n      - ", job) if re.match(rf"(?:if: .*\n\s*)?uses: {re.escape(reference)}\n", text)]
    if len(steps) != 1:
        raise AssertionError(f"Expected one step using {reference!r}, found {len(steps)}")
    return steps[0]


def step(job, name):
    """The text of the job's step with this exact name."""
    steps = [text for text in re.split(r"\n      - ", job) if re.match(rf"(?:if: .*\n\s*)?name: {re.escape(name)}\n", text)]
    if len(steps) != 1:
        raise AssertionError(f"Expected one step named {name!r}, found {len(steps)}")
    return steps[0]


def plain_scalar_faults(text):
    """Unquoted one-line values that YAML would not read as plain text.

    The standard library has no YAML parser, and GitHub rejects the whole workflow,
    running no job at all, when one value is not valid YAML. A plain (unquoted) value
    must not contain ": " or end in ":" (both start a mapping). Block scalars
    ("run: |") and quoted, flow and anchored values are not plain and are skipped.
    """
    faults = []
    block_indent = None
    for number, line in enumerate(text.splitlines(), 1):
        indent = len(line) - len(line.lstrip(" "))
        if block_indent is not None:
            if not line.strip() or indent > block_indent:
                continue
            block_indent = None
        match = re.match(r"^\s*(?:- )?[A-Za-z0-9_-]+:(?: (.*))?$", line)
        if not match or match.group(1) is None:
            continue
        value = match.group(1).split(" #", 1)[0].rstrip()
        if value[:1] in ("|", ">"):
            block_indent = indent
            continue
        if not value or value[0] in "'\"[{&*!%@`":
            continue
        if ": " in value or value.endswith(":"):
            faults.append(f"line {number}: {line.strip()}")
    return faults


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.text = WORKFLOW.read_text(encoding="utf-8")
        self.jobs = jobs(self.text)
        self.workflows = {path.name: path.read_text(encoding="utf-8") for path in WORKFLOWS}

    def test_archive_job_opts_into_plugin_tests_and_a_release_package_on_both_platforms(self):
        job = self.jobs["archive-filesystem"]
        self.assertRegex(job, r'scripts/build\.ps1" .*-Profile release -Test')
        self.assertRegex(job, r'bash "\$plugin/scripts/build\.sh" \$sdk \$build \$package release --test')

    def test_every_job_is_bounded_and_installs_the_pinned_toolchain_explicitly(self):
        self.assertIn("ci.yml", self.workflows)
        for workflow, text in self.workflows.items():
            self.assertNotIn("rustup show", text)
            for name, job in jobs(text).items():
                with self.subTest(workflow=workflow, job=name):
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

    def test_the_opt_in_acceptance_suite_is_checked_on_both_platforms(self):
        # `--workspace --all-targets` skips a target whose required feature is off, so
        # the workspace step enables it. A separate `-p midnight-acceptance` step would
        # resolve features without the workspace's dev-dependencies and check tokio and
        # everything above it twice.
        lint = self.jobs["scripts-and-lint"]
        self.assertIn("os: [windows-latest, ubuntu-24.04]", lint)
        text = step(lint, "Clippy")
        self.assertIn(
            "cargo clippy --workspace --all-targets --locked "
            "--features midnight-acceptance/acceptance -- -D warnings",
            text,
        )
        self.assertNotIn("if:", text)
        commands = [line for line in self.text.splitlines() if not line.lstrip().startswith("#")]
        root = [line for line in commands if "cargo clippy" in line and "--manifest-path" not in line]
        self.assertEqual(len(root), 1, "one root-workspace Clippy step, one feature resolution")
        self.assertNotIn("-p midnight-acceptance", "\n".join(commands))

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

    def test_release_job_validates_the_filesystem_packages_as_packaging_does(self):
        job = self.jobs["release"]
        packages = job.index("uses: ./.github/actions/plugin-packages")
        validation = job.index("scripts/package-release.py validate-filesystems --source _build/filesystem-packages")
        self.assertLess(packages, validation)
        self.assertIn("'x86_64-pc-windows-msvc'", job)
        self.assertIn("'x86_64-unknown-linux-gnu'", job)

    def test_release_job_installs_the_msi_for_real_only_on_the_disposable_windows_runner(self):
        job = self.jobs["release"]
        self.assertIn("os: [windows-latest, ubuntu-24.04]", job)
        built = step(job, "MSI of a release-shaped ZIP of this build and its ICE validation")
        published = step(job, "Published per-user MSIs for the upgrade test")
        installed = step(job, "MSI install, upgrade, downgrade, repair and removal in both scopes")
        dialogs = step(job, "MSI dialogs driven by UI Automation")
        for text in (built, published, installed, dialogs):
            self.assertIn("if: runner.os == 'Windows'\n", text)
        # Built from this run's release executable and plugin packages, after they arrive.
        self.assertLess(job.index("uses: ./.github/actions/plugin-packages"), job.index(built))
        for argument in ("--executable _build/release/midnight-explorer.exe", "--viewers _build/viewer-packages",
                         "--filesystems _build/filesystem-packages", "test-msi-install.py prepare",
                         "test-msi-install.py ice"):
            self.assertIn(argument, built)
        self.assertIn("GH_TOKEN: ${{ github.token }}", published)
        for argument in ("--disposable-machine", "--scenario scopes", "--scenario previous", "--ui-smoke"):
            self.assertIn(argument, installed)
        self.assertNotIn("continue-on-error", installed)
        self.assertIn("continue-on-error: true", dialogs)
        self.assertIn("--disposable-machine --work _build/msi-test/work --scenario ui", dialogs)
        logs = uses(job, "actions/upload-artifact@v7")
        self.assertIn("if: always() && runner.os == 'Windows'", logs)
        self.assertIn("path: _build/msi-test/work/run-*/logs", logs)
        # The disposable-runner flag appears nowhere else.
        self.assertEqual(self.text.count("--disposable-machine"), 2)

    def test_clippy_receives_the_sdk_configuration(self):
        # Cargo does not pass options given before the subcommand to an external
        # subcommand: `cargo --config X clippy` resolves without X (the video platform
        # helpers' job then found no `midnight` SDK package).
        for name, job in self.jobs.items():
            with self.subTest(job=name):
                self.assertNotRegex(job, r"cargo\s+(?:\+\S+\s+)?--config\s+\S+\s+clippy\b")
        self.assertIn("cargo clippy --config $sdk", self.jobs["thumbnail-video-platform"])

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
        self.assertIn('archive-fixtures: "true"', uses(check, "./.github/actions/plugin-packages"))
        self.assertIn("name: archive-fixtures-${{ runner.os }}", action("plugin-packages"))
        text = step(check, "Installed archive worker streaming, all formats, cooperative cancellation and package discovery")
        for variable in ("ARCHIVE_CAPTURE_INSTALLATION", "MIDNIGHT_ARCHIVE_FIXTURES", "MIDNIGHT_ARCHIVE_TEST_ZIP"):
            self.assertIn(f"$env:{variable} = ", text)
        self.assertIn("scripts/archive-test-zip.py", text)
        for selection in ("test:streaming_worker --ignored", "lib captured_source_tests:: --ignored",
                          "test:discovery_kinds --ignored"):
            self.assertIn(f"run midnight-filesystem-host {selection}", text)
        nested = step(check, "Restricted archive workers open archives inside archives")
        self.assertIn("run midnight-filesystem-host test:nested_worker --ignored --test-threads=1", nested)

    def test_workspace_jobs_start_at_once_and_wait_for_plugin_packages_only_before_using_them(self):
        # The plugin jobs take up to about 11 minutes; check and release build meanwhile.
        for name in ("scripts-and-lint", "check", "release"):
            with self.subTest(job=name):
                job = self.jobs[name]
                self.assertNotIn("\n    needs:", job)
                self.assertNotIn("actions/download-artifact", job, "only through plugin-packages")
        for name, build in (("check", "Test"), ("release", "Optimized release build")):
            with self.subTest(job=name):
                job = self.jobs[name]
                self.assertIn("runner-label: ${{ matrix.os }}", uses(job, "./.github/actions/plugin-packages"))
                packages = job.index("uses: ./.github/actions/plugin-packages")
                self.assertLess(job.index("uses: ./.github/actions/private-test-installation"), job.index(f"name: {build}\n"))
                self.assertLess(job.index(f"name: {build}\n"), packages)
                # Steps after the action may use the packages; none before it may.
                before = job[:packages]
                for value in ("HOST_TEST_PLUGIN", "MIDNIGHT_VIEWER_TEST_INSTALL", "MIDNIGHT_NATIVE_TEST_INSTALLATION",
                              "ARCHIVE_WORKER", "ARCHIVE_TEST_INSTALL_ROOT", "_build/viewer-packages",
                              "_build/filesystem-packages", "_build/archive-fixtures", "--ignored", "--smoke-test"):
                    self.assertNotIn(value, before)
        self.assertIn("--smoke-test", self.jobs["release"].split("uses: ./.github/actions/plugin-packages", 1)[1])
        # The early action creates only the protected root; plugin-packages installs into it.
        early = action("private-test-installation")
        self.assertIn("HOST_TEST_INSTALL_ROOT=", early)
        self.assertNotIn("_build/viewer-packages", early)
        installing = action("plugin-packages")
        for variable in ("HOST_TEST_PLUGIN", "MIDNIGHT_VIEWER_TEST_INSTALL", "MIDNIGHT_NATIVE_TEST_INSTALLATION",
                         "MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION", "MIDNIGHT_PDF_VIEWER_TEST_APPLICATION",
                         "ARCHIVE_WORKER", "ARCHIVE_TEST_INSTALL_ROOT"):
            self.assertIn(f'"{variable}=', installing)

    def test_plugin_packages_wait_for_every_artifact_that_their_downloads_select(self):
        packages = action("plugin-packages")
        def matrix(job, key):
            values = re.search(rf"^        {key}: \[([^\]]*)\]$", self.jobs[job], re.MULTILINE).group(1)
            return {value.strip() for value in values.split(",")}
        def listed(prefix):
            names = re.search(rf"@\(([^)]*) \| ForEach-Object {{ \"{prefix}-\$os-\$_\" }}\)", packages).group(1)
            return set(re.findall(r"'([^']+)'", names))
        self.assertEqual(listed("viewer"), matrix("viewers", "plugin") | matrix("image-viewer", "plugin"))
        providers = set(re.findall(r"^            provider: (\S+)$", self.jobs["filesystem-libraries"], re.MULTILINE))
        self.assertEqual(listed("filesystem"), providers | {"archive"})
        self.assertIn('$names += "archive-fixtures-$os"', packages)
        # The jobs that upload these artifacts are the producers whose failure ends the wait.
        producers = {name for name, job in self.jobs.items()
                     if re.search(r"name: (viewer|filesystem|archive-fixtures)-\$\{\{ runner\.os \}\}", job)}
        self.assertEqual(set(re.findall(r"--producer (\S+)", packages)), producers)
        for name in producers:
            with self.subTest(producer=name):
                # Legs are matched by the job name "<job> (<os>, ...)" and the runner label.
                self.assertRegex(self.jobs[name], r"matrix:\n        (os:|include:\n          - os:)")
                self.assertIn("runs-on: ${{ matrix.os }}", self.jobs[name])
                self.assertNotRegex(self.jobs[name], r"(?m)^    name:", "a job name replaces the matrix values")
        for pattern in ("pattern: viewer-${{ runner.os }}-*", "pattern: filesystem-${{ runner.os }}-*"):
            self.assertIn(pattern, packages)
        self.assertLess(packages.index("scripts/ci-wait-for-artifacts.py"),
                        packages.index("uses: ./.github/actions/download-artifacts"))
        self.assertIn("GH_TOKEN: ${{ github.token }}", packages)

    def test_artifacts_are_extracted_by_the_checking_extractor_not_by_download_artifact(self):
        # download-artifact's own unzip dependency calls the deprecated Buffer() constructor
        # (Node DEP0005, actions/download-artifact#484); it still checks the SHA-256 digest.
        local = {path.parent.name: path.read_text(encoding="utf-8")
                 for path in (ROOT / ".github/actions").glob("*/action.yml")}
        downloads = local.pop("download-artifacts")
        for name, text in [*self.workflows.items(), *local.items()]:
            with self.subTest(file=name):
                self.assertNotIn("uses: actions/download-artifact@", text)
                for entry in re.split(r"\n\s+- ", text):
                    if "uses: ./.github/actions/download-artifacts\n" in entry:
                        self.assertIn("path: _build/", entry)
                        self.assertEqual(len(re.findall(r"\n\s+(name|pattern): \S", entry)), 1, entry)
                        self.assertNotIn("merge-multiple", entry)
        self.assertEqual(downloads.count("uses: actions/download-artifact@"), 1)
        self.assertIn("skip-decompress: true", downloads)
        self.assertNotIn("digest-mismatch", downloads, "the default fails on a digest mismatch")
        self.assertNotIn("merge-multiple", downloads, "one directory per artifact; the extractor merges")
        self.assertLess(downloads.index("uses: actions/download-artifact@"),
                        downloads.index("EXTRACTOR: ${{ github.action_path }}/extract.py"))
        self.assertIn("Give exactly one of the name and pattern inputs", downloads)
        self.assertTrue((ROOT / ".github/actions/download-artifacts/extract.py").is_file())

    def test_only_the_waiting_jobs_may_read_the_runs_jobs_and_artifacts(self):
        for workflow, text in self.workflows.items():
            permissions = text.split("\npermissions:\n", 1)[1].split("\n\n", 1)[0]
            self.assertEqual(permissions.strip(), "contents: read", workflow)
            for name, job in jobs(text).items():
                with self.subTest(workflow=workflow, job=name):
                    if "uses: ./.github/actions/plugin-packages" in job:
                        self.assertIn("    permissions:\n      contents: read\n      actions: read\n", job)
                    else:
                        self.assertNotIn("\n    permissions:", job)

    def test_tests_and_clippy_build_alike_on_every_cpu_with_the_default_linker(self):
        def environment(job):
            block = re.search(r"\n    env:\n((?:      .*\n)+)", self.jobs[job]).group(1)
            return [line.strip() for line in block.splitlines() if not line.strip().startswith("#")]
        check = environment("check")
        for setting in ('CARGO_PROFILE_DEV_DEBUG: "0"', 'CARGO_PROFILE_TEST_DEBUG: "0"', 'CARGO_INCREMENTAL: "0"'):
            self.assertIn(setting, check)
        # Same settings, so the two jobs' dependency caches hold the same builds.
        self.assertEqual(environment("scripts-and-lint"), check)
        # Cargo, libtest and the test runner default to every CPU; nothing caps them here
        # or in the Cargo configurations (docs/verification/2026-11-21-ci-time.md).
        self.assertNotIn("CARGO_BUILD_JOBS", self.text)
        self.assertNotRegex(self.text, r"ci-workspace-tests\.py[^\n]*--jobs")
        for config in (ROOT / ".cargo/config.toml", ROOT / "api/.cargo/config.toml"):
            self.assertNotRegex(config.read_text(encoding="utf-8"), r"(?m)^jobs\s*=", config)
        # rust-lld made no measurable difference on the hosted runners; release builds
        # keep the MSVC linker in any case.
        self.assertNotIn("LINKER", self.text)
        self.assertNotIn("linker", (ROOT / ".cargo/config.toml").read_text(encoding="utf-8"))

    def test_only_root_workspace_jobs_cache_build_output_and_only_main_saves_it(self):
        for workflow, text in self.workflows.items():
            for name, job in jobs(text).items():
                with self.subTest(workflow=workflow, job=name):
                    cached = 'build-cache: "true"' in job
                    self.assertEqual(cached, workflow == "ci.yml" and name in ("scripts-and-lint", "check", "release"))
        setup = action("rust-setup")
        self.assertRegex(setup, r"uses: Swatinem/rust-cache@[0-9a-f]{40} # v\d+\.\d+\.\d+\n")
        self.assertIn('workspaces: ". -> _build"', setup)
        self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' }}", setup)
        self.assertIn("if: inputs.build-cache != 'true' && inputs.download-cache == 'true'\n      uses: actions/cache@v6", setup)
        self.assertRegex(setup, r'  download-cache:\n    description: >-\n(?:      .*\n)+    default: "true"\n')

    def test_github_actions_run_on_node_24_with_one_version_each(self):
        # Their Node 20 majors print deprecation notices and DeprecationWarnings on the
        # hosted runners; these are the first majors that run on Node 24 by default.
        node24 = {"checkout": 5, "cache": 5, "upload-artifact": 6, "download-artifact": 7}
        texts = [*self.workflows.values()] + [path.read_text(encoding="utf-8")
                                              for path in sorted((ROOT / ".github/actions").glob("*/action.yml"))]
        used = [(name, int(major)) for text in texts
                for name, major in re.findall(r"uses: actions/([a-z-]+)@v(\d+)\n", text)]
        self.assertEqual({name for name, _ in used}, set(node24))
        for name, minimum in node24.items():
            with self.subTest(action=name):
                majors = {major for used_name, major in used if used_name == name}
                self.assertEqual(len(majors), 1, f"one {name} version everywhere: {majors}")
                self.assertGreaterEqual(majors.pop(), minimum)

    @unittest.skipUnless(importlib.util.find_spec("yaml"), "PyYAML is not installed")
    def test_workflow_and_local_actions_parse_with_a_consistent_structure(self):
        import yaml
        for workflow, text in self.workflows.items():
            parsed = yaml.safe_load(text)["jobs"]
            self.assertEqual(set(parsed), set(jobs(text)), workflow)
            for name, job in parsed.items():
                with self.subTest(workflow=workflow, job=name):
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


    def test_one_line_values_are_valid_plain_yaml(self):
        # A plain value with ": " made GitHub reject the workflow (no job ran) while the
        # PyYAML parse below was skipped locally; this check needs no YAML parser.
        self.assertEqual(plain_scalar_faults("run: cargo test x:: --ignored\n"),
                         ["line 1: run: cargo test x:: --ignored"])
        self.assertEqual(plain_scalar_faults("run: |\n  echo a: b\nname: 'q: x'\n"), [])
        for path in [*WORKFLOWS, *sorted((ROOT / ".github/actions").glob("*/action.yml"))]:
            with self.subTest(path=path.relative_to(ROOT).as_posix()):
                self.assertEqual(plain_scalar_faults(path.read_text(encoding="utf-8")), [])

    def test_the_dependency_inventory_is_captured_on_each_host_and_regenerated(self):
        # Build dependencies and procedural macros follow the host (docs/THIRD_PARTY.md):
        # a Windows capture taken on Linux once listed Linux-only build dependencies.
        text = INVENTORY_WORKFLOW.read_text(encoding="utf-8")
        inventory = jobs(text)
        on = text.split("\non:\n", 1)[1].split("\n\n", 1)[0]
        self.assertRegex(on, r"push:\n    branches: \[main\]\n    paths:\n")
        self.assertIn("\n  workflow_dispatch:", on)
        for path in ("**/Cargo.toml", "**/Cargo.lock", ".cargo/config.toml", "rust-toolchain.toml",
                     "docs/dependencies/**", "scripts/capture-dependency-inventory.ps1",
                     "scripts/dependency-inventory.py", "scripts/package-release.py",
                     ".github/workflows/dependency-inventory.yml"):
            with self.subTest(path=path):
                self.assertEqual(on.count(f'      - "{path}"\n'), 2, "for pushes and pull requests alike")
        capture = inventory["capture"]
        self.assertIn("os: [windows-latest, ubuntu-24.04]", capture)
        self.assertIn("runs-on: ${{ matrix.os }}", capture)
        self.assertIn("./scripts/capture-dependency-inventory.ps1 -OutputDirectory _build/dependency-captures", capture)
        self.assertIn("-IncludeMacOS:$IsLinux", capture)
        self.assertIn("name: dependency-captures-${{ runner.os }}", capture)
        regenerate = inventory["inventory"]
        self.assertIn("pattern: dependency-captures-*", regenerate)
        self.assertIn("scripts/dependency-inventory.py --metadata-dir _build/dependency-captures", regenerate)
        # Stale tables fail the run, and the regenerated ones are still uploaded.
        compare = step(regenerate, "Regenerate the inventory and compare it with docs/dependencies")
        self.assertIn("::error title=Dependency inventory::", compare)
        commands = [line.strip() for line in compare.splitlines() if line.strip() and not line.strip().startswith("#")]
        self.assertEqual(commands[-1], "exit 1")
        self.assertIn("if: ${{ !cancelled() }}\n        uses: actions/upload-artifact@v7", regenerate)
        self.assertIn("name: dependency-inventory\n", uses(regenerate, "actions/upload-artifact@v7"))
        for name, job in inventory.items():
            if "uses: ./.github/actions/rust-setup" in job:
                with self.subTest(job=name):
                    self.assertIn('download-cache: "false"', uses(job, "./.github/actions/rust-setup"))


if __name__ == "__main__":
    unittest.main()
