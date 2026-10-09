#!/usr/bin/env python3
"""Check runner failure, report lifetime, selection and version contracts."""

import importlib.util
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / ".config" / "nextest.toml"
spec = importlib.util.spec_from_file_location("test_rust", ROOT / "scripts/test-rust.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class WrapperContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="emuella-runner-contract-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.report = self.root / ".test-results/nextest/default/junit.xml"
        self.report.parent.mkdir(parents=True)
        self.report.write_text("stale success", encoding="utf-8")
        self.root_patch = patch.object(runner, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def invoke(self, statuses):
        calls = []

        def run(argv, cwd):
            self.assertEqual(cwd, self.root)
            # Cleanup must precede the first setup/build command.
            self.assertFalse(self.report.exists())
            calls.append(argv)
            return subprocess.CompletedProcess(argv, statuses[len(calls) - 1])

        with patch.object(runner.subprocess, "run", side_effect=run):
            status = runner.main([])
        return status, calls

    def test_preflight_failure_removes_stale_report_and_stops(self):
        status, calls = self.invoke([101])
        self.assertEqual(status, 101)
        self.assertEqual(calls, [["cargo", "nextest", "show-config", "version"]])

    def test_test_failure_propagates_without_doctests(self):
        status, calls = self.invoke([0, 100])
        self.assertEqual(status, 100)
        self.assertEqual(calls[-1], [
            "cargo", "nextest", "run", "--workspace", "--profile", "default",
        ])
        self.assertEqual(len(calls), 2)

    def test_doctests_are_separate_and_propagate_failure(self):
        status, calls = self.invoke([0, 0, 101])
        self.assertEqual(status, 101)
        self.assertEqual(calls[-1], ["cargo", "test", "--workspace", "--doc"])

    def test_prepare_removes_only_the_selected_report_without_setup(self):
        peer = self.root / ".test-results/nextest/ci/junit.xml"
        peer.parent.mkdir(parents=True)
        peer.write_text("peer report", encoding="utf-8")
        with patch.object(runner.subprocess, "run") as run:
            self.assertEqual(runner.main(["--prepare"]), 0)
            run.assert_not_called()
        self.assertFalse(self.report.exists())
        self.assertEqual(peer.read_text(encoding="utf-8"), "peer report")


class NextestContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="emuella-nextest-contract-")
        cls.root = Path(cls.temporary.name)
        (cls.root / "src").mkdir()
        (cls.root / "Cargo.toml").write_text(
            '[package]\nname = "emuella-runner-probe"\nversion = "0.0.0"\n'
            'edition = "2024"\n\n[workspace]\n', encoding="utf-8",
        )
        (cls.root / "src/lib.rs").write_text(
            '#[test]\nfn a_fails() { panic!("authored runner failure probe"); }\n'
            '#[test]\nfn z_continues() {}\n'
            '#[test]\n#[ignore]\nfn ignored_probe() { panic!("must stay ignored"); }\n',
            encoding="utf-8",
        )
        cls.env = os.environ.copy()
        cls.env["CARGO_TARGET_DIR"] = str(cls.root / "target")
        cls.env["CARGO_TERM_COLOR"] = "never"
        cls.env["NEXTEST_USER_CONFIG_FILE"] = "none"

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def run_nextest(self, *args, config=CONFIG):
        result = subprocess.run(
            ["cargo", "nextest", *args, "--manifest-path", str(self.root / "Cargo.toml"),
             "--config-file", str(config)],
            # Keep the repository's effective Rust toolchain; only the probe's
            # manifest, build output and reports belong to the temporary project.
            cwd=ROOT, env=self.env, text=True, capture_output=True,
        )
        return result, result.stdout + result.stderr

    def cases(self, profile):
        report = self.root / ".test-results/nextest" / profile / "junit.xml"
        self.assertTrue(report.is_file(), report)
        return {case.attrib["name"]: case for case in ET.parse(report).iter("testcase")}

    def test_ci_continues_after_failure_without_retry_and_reports_ignored(self):
        result, output = self.run_nextest(
            "run", "--workspace", "--profile", "ci", "--test-threads", "1",
        )
        self.assertEqual(result.returncode, 100, output)
        cases = self.cases("ci")
        self.assertEqual(set(cases), {"a_fails", "z_continues", "ignored_probe"})
        self.assertIsNotNone(cases["a_fails"].find("failure"))
        self.assertIsNone(cases["a_fails"].find("rerunFailure"))
        self.assertIsNone(cases["z_continues"].find("failure"))
        self.assertIsNone(cases["z_continues"].find("skipped"))
        self.assertIsNotNone(cases["ignored_probe"].find("skipped"))

    def test_exact_filter_and_default_profile_report(self):
        result, output = self.run_nextest(
            "run", "--workspace", "--profile", "default", "-E", "test(=z_continues)",
        )
        self.assertEqual(result.returncode, 0, output)
        cases = self.cases("default")
        self.assertEqual(set(cases), {"a_fails", "z_continues", "ignored_probe"})
        self.assertIsNone(cases["z_continues"].find("skipped"))
        self.assertIsNotNone(cases["a_fails"].find("skipped"))
        self.assertIsNotNone(cases["ignored_probe"].find("skipped"))

    def test_valid_higher_minimum_produces_a_version_diagnostic(self):
        version = subprocess.run(
            ["cargo", "nextest", "--version"], cwd=ROOT, env=self.env,
            text=True, capture_output=True, check=True,
        ).stdout
        match = re.search(r"cargo-nextest (\d+)\.(\d+)\.(\d+)", version)
        self.assertIsNotNone(match, version)
        major, minor, patch_version = map(int, match.groups())
        higher = f"{major}.{minor}.{patch_version + 1}"
        config = self.root / "higher-minimum.toml"
        config.write_text(
            re.sub(
                r"nextest-version = .*",
                f'nextest-version = {{ required = "{higher}", recommended = "{higher}" }}',
                CONFIG.read_text(encoding="utf-8"), count=1,
            ), encoding="utf-8",
        )
        # required == recommended is valid; this must reject the installed runner,
        # rather than accidentally reject an inconsistent configuration.
        result, output = self.run_nextest("show-config", "version", config=config)
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn(higher, output)
        self.assertIn("required", output.lower())
        self.assertNotIn("error parsing", output.lower())

    def test_warning_and_retry_configuration_preserves_diagnostics(self):
        config = tomllib.loads(CONFIG.read_text(encoding="utf-8"))
        self.assertEqual(config["profile"]["default"]["slow-timeout"], "5s")
        self.assertEqual(config["profile"]["default"]["retries"], 0)
        self.assertFalse(config["profile"]["ci"]["fail-fast"])
        self.assertEqual(config["nextest-version"], {
            "required": "0.9.146", "recommended": "0.9.146",
        })


if __name__ == "__main__":
    unittest.main(verbosity=2)
