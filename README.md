# emuella-testdata

`emuella-testdata` is the reproducible test-data catalogue for the Emuella
image codec projects. It separates mixed-license test imagery from the
Apache-2.0 codec implementations while giving tests and benchmarks stable pack
identities, provenance, integrity checks, and opt-in materialization.

The Git repository contains no third-party image or conformance binaries. Its
software, documentation, manifests, schemas, recipes, and project-generated
fixtures are Apache-2.0 unless a file explicitly says otherwise. Material
obtained from an external pack remains under that pack's own terms.

## Test layers

1. **Layer 1 — self-contained unit fixtures.** Tiny, deterministic,
   project-authored cases live with a codec or in `common/generated-core`.
   Ordinary `cargo test` never needs network access or third-party data.
2. **Layer 2 — optional corpus integration.** Conformance, interoperability,
   benchmark, and robustness tests consume independently materialized packs by
   stable ID from a caller-provided cache.
3. **Layer 3 — pinned CI qualification.** CI records the codec commit, catalogue
   commit, pack version, byte digest, harness version, and execution profile.
   Restricted packs are pre-provisioned rather than silently downloaded.

See [Architecture](docs/ARCHITECTURE.md) and
[main-repository integration](docs/MAIN_REPO_INTEGRATION.md) for the complete
model.

## Repository map

- `crates/emuella-corpus`: catalogue validation, fixture generation, and pack
  verification CLI.
- `manifests`: one independently licensed and versioned record per corpus pack.
- `inventories`: complete, deterministic file records for locked external packs.
- `suites`: named selections used by local testing and CI.
- `generated`: committed Apache-2.0 fixtures created by the CLI.
- `recipes`: human- and machine-readable transformation provenance.
- `schema`: JSON Schemas for editor and external-tool integration.
- `LICENSES`: licence references used by external manifests; these do not
  relicense upstream assets.

## Commands

Run commands from the repository root, or set `EMUELLA_TESTDATA_ROOT`:

```sh
cargo run -p emuella-corpus -- list packs
cargo run -p emuella-corpus -- list suites
cargo run -p emuella-corpus -- show common/generated-core
cargo run -p emuella-corpus -- check
cargo run -p emuella-corpus -- verify common/generated-core
cargo run -p emuella-corpus -- verify jpeg-2000/conformance
cargo run -p emuella-corpus -- inventory PACK --root PATH --output PATH
cargo run -p emuella-corpus -- generate common/generated-core --output /tmp/generated-core
```

`check` validates all manifests, suite references, local paths, licence-review
states, and digest syntax. `verify` compares a materialized pack with its
recorded file sizes and SHA-256 digests, then verifies the complete tree digest
when one is present. `inventory` is a maintainer command: it records every
regular file and rejects symlinks or other special filesystem entries.

The CLI deliberately does not accept external licence terms or download
click-through material on a user's behalf. External manifests provide the
authoritative source and terms links, expected layout, and lock status.

## Local checks

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p emuella-corpus -- check
cargo run -p emuella-corpus -- verify common/generated-core
./scripts/check-generated.sh
python3 recipes/check-independent-nitf.py
cargo deny check
```

The small independent NITF/JPEG 2000 pack includes native 11-bit PAN, U16 grey,
RGB8 and RGB16 arithmetic fixtures. Its
[recipe and verification instructions](recipes/independent-nitf-v1.md) describe
optional complete OpenJPEG CLI decoding, pinned regeneration and bounded-tile
large-source generation. The ordinary catalogue checks need no GDAL bindings
or NumPy; regeneration requires NumPy and an explicitly selected GDAL shared
library with OpenJPEG support.

The local-only [RarePlanes calibration recipe](recipes/rareplanes-calibration-v1.md)
prepares two authorised full-location PAN, RGB and native multispectral bundles,
with exact source locks and independent full-sample verification. GDAL/NumPy
are optional preparation dependencies; ordinary catalogue checks stay offline.

The separate [twelve-location RarePlanes recipe](recipes/rareplanes-expanded-v1.md)
freezes metadata coverage before timing and prepares 48 complete sample arrays.
Its source locks and selection remain separate from the original calibration.

## Licensing

There is intentionally no single licence for every materialized corpus. Read
[LICENSING.md](LICENSING.md) before adding a pack or publishing derived assets.
Unknown rights are treated as unavailable: they do not imply permission to
redistribute, modify, use commercially, train models, or publish derivatives.
