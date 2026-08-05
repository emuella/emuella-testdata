# Integrating a codec repository

## Layer 1

Keep ordinary unit tests entirely inside the codec repository and under its
Apache-2.0 scope. Tests may use tiny handwritten bytes or project-generated
fixtures. They must not require `emuella-testdata`, a sibling checkout, or a
network connection.

## Layer 2

Add an opt-in harness or `xtask` that accepts, in priority order:

1. an explicit `--testdata` directory;
2. `EMU_TESTDATA_CACHE`; or
3. an already materialized sibling catalogue cache.

The harness should report a skipped suite when data is absent, not silently
download it. A separate user command may invoke the catalogue tool and review
the relevant terms before materialization.

Do not embed external files with `include_bytes!`, copy them into crate test
directories, or make a test-data repository a Cargo dependency. The harness
should consume paths at runtime.

## Layer 3

Commit a small `testdata.lock.toml` to the codec repository. It should contain
only catalogue and pack identities:

```toml
schema_version = 1
catalogue_commit = "<full Git commit>"

[[packs]]
id = "common/generated-core"
version = "1"

[[packs]]
id = "jpeg-xl/conformance"
version = "18181-3-2025-ed2"
archive_sha256 = "<locked digest>"
```

CI checks out the pinned catalogue commit, verifies pre-provisioned or
explicitly acquired data, and records the lockfile with its result. Never use a
floating catalogue branch for a release gate.

## Packaging guardrail

Before publishing a crate or source archive, inspect `cargo package --list`
and the final archive. Cache directories, external golden outputs, conformance
attachments, and third-party derivatives must not appear.

## Result handling

Decoded pixels and recompressed images derived from third-party sources remain
test-data artifacts. Keep them outside the codec repository. Factual results
such as hashes, dimensions, error metrics, timings, and pass/fail states may be
stored in the codec's CI system or benchmark history with the required
attribution and pack identity.
