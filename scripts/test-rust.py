#!/usr/bin/env python3
"""Run the catalogue's ordinary Rust tests and separate Cargo doctests."""

import argparse
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("default", "ci"), default="default")
    parser.add_argument(
        "--prepare", action="store_true",
        help="remove the owned report before CI toolchain and runner setup",
    )
    args = parser.parse_args(argv)

    # Reports survive Cargo target cleanup; remove only this run's owned file.
    report = ROOT / ".test-results" / "nextest" / args.profile / "junit.xml"
    report.unlink(missing_ok=True)
    if args.prepare:
        return 0

    preflight = subprocess.run(
        ["cargo", "nextest", "show-config", "version"], cwd=ROOT,
    )
    if preflight.returncode:
        print(
            "Install cargo-nextest 0.9.146 or newer using the pre-built binaries: "
            "https://nexte.st/docs/installation/pre-built-binaries/",
            file=sys.stderr,
        )
        return preflight.returncode

    tests = subprocess.run(
        ["cargo", "nextest", "run", "--workspace", "--profile", args.profile],
        cwd=ROOT,
    )
    if tests.returncode:
        return tests.returncode
    return subprocess.run(["cargo", "test", "--workspace", "--doc"], cwd=ROOT).returncode


if __name__ == "__main__":
    sys.exit(main())
