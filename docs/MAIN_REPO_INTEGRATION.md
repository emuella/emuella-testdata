# Integrating a codec repository

## Layer 1

Keep ordinary unit tests entirely inside the codec repository and under its
Apache-2.0 scope. Tests may use tiny handwritten bytes or project-generated
fixtures. They must not require `emuella-testdata`, a sibling checkout, or a
network connection.

## Layer 2

Add an opt-in harness or `xtask` that accepts, in priority order:

1. an explicit `--testdata` directory;
2. `EMUELLA_TESTDATA_CACHE`; or
3. an already materialized sibling catalogue cache.

The harness should report a skipped suite when data is absent, not silently
download it. A separate user command may invoke the catalogue tool and review
the relevant terms before materialization.

When a suite supplies an `inspection` plan, select candidates from the named
pack's locked inventory by its declared extensions. Apply exactly one path or
path-prefix classification to every candidate, then apply any per-path outcome
override. `emuella-corpus check` rejects empty selections, unclassified or
multiply classified candidates, dead classification rules, duplicate or
unselected overrides, and rejection expectations without a diagnostic. Codec
harnesses should consume this catalogue-owned format, cohort, and acceptance
contract rather than infer it from file names or observed codec behaviour.

When a suite supplies a `decoded_pixel_comparison` plan, consume only its
explicit locked-pack cases. Each case binds one codestream to one PGX component
reference, its logical dimensions and sample format, and inclusive peak-error
and mean-squared-error limits. Validate both paths against the locked inventory
before opening either asset. Compare logical component samples after any
required output normalisation; file-byte equality is not the comparison
contract. Keep protected input, reference, and decoded samples in the
authorised store and process memory, and report only factual identities,
dimensions, pass/fail state, and aggregate errors.

When a suite supplies a `rendered_pixel_comparison` plan, hand it to a
dedicated rendered-output codec worker or runner rather than the native PGX
comparison path. The initial Annex G case is full-frame only: decode the named
JP2 through the codec's rendered API, compare its 8-bit sRGB output with the
named contiguous RGB TIFF, and apply the inclusive aggregate peak-error limit.
The plan intentionally has no component selector, native reduction or region,
mean-squared-error bound, interpolation parameters, decoded digest or pixel
payload. Resolve both paths from the selected locked inventory before opening
them, keep protected and derived pixels in the authorised store and process
memory, and report only pack and case identities, dimensions, pass/fail state
and aggregate peak error. Missing data must skip or fail according to the suite
policy; the worker must not acquire it, copy it into a fallback location or
fall back to a different decode route.

Treat schema validation as a portable shape check, not as inventory or
cross-record validation. `emuella-corpus check` additionally proves that the
selected pack is locked, each path is an exact inventory member, the input and
reference are semantically distinct, and rendered case IDs, inputs and
references are unique across the plan. Consumers should fail closed if either
layer rejects the plan.

When one codestream has alternative authoritative outputs, consume its
input-level `choice_group`. Each alternative independently declares its
reference, component, resolution reduction, comparison-window origin in that
selected output resolution, logical format, and limits. Window width and height
come from the declared logical dimensions. The group's
`minimum_passing_alternatives` value defines how many alternatives must pass;
alternatives are choices, not implicitly cumulative requirements. The current
choice-group contract accepts only the zero- and one-level reductions required
by P0.03 and P0.15; broader reduction contracts require a separate schema
change.

When the plan supplies a `derived_set`, match its declared set, profile,
compliance class, and coding mode to the decoder capability being qualified.
For each case point, use the caller-declared `M_MAGB` capability to select the
variant with the greatest `B_MAGB` that does not exceed it. A case is not
applicable when no variant satisfies that rule; do not substitute the nearest
higher variant. `M_MAGB` belongs to the decoder claim and is deliberately not
fixed by the catalogue, even when a particular qualification run uses 18.

Each derived-set case resolves its `reference_case_id` to exactly one scalar
case or choice group in the same plan. The selected variant then supplies the
final inclusive peak-error and mean-squared-error limits for every required
component or every reference alternative. These are complete comparison
limits, including any applicable Part 4 addition, rather than deltas for the
harness to reinterpret. Apply the plan's conditional output-normalisation
steps before comparison: order-dependent steps remain ordered, while the
order-independent steps may be applied in any convenient order. The catalogue
step to recover the first codestream component includes reversing decoded
ICT/RCT output when necessary; it does not mean selecting the first
display-colour component. The catalogue contract describes standards-owned
inputs and acceptance bounds; it does not predict whether a particular codec
will pass or reject them.

For suite schema version 1 compatibility, plans without `derived_sets` may omit
`output_normalisation`; existing scalar-only consumers therefore remain valid.
A non-empty `derived_sets` array requires the complete normalisation contract,
and catalogue validation fails closed when it is absent or altered.

The DS0 contract is based on ISO/IEC 15444-4:2024 | ITU-T T.803 (V3), B.2 and
B.2.2 to B.2.5 (PDF pages 24 to 26), C.2.1 and Tables C.1 and C.1bis (PDF pages
31 and 32), at retrieval revision
`725ecba70e5d03eff3f6ce9626bb9cb08dd4e0c7` and reviewed bundle revision
`7b3d8d60cd4d4f6c056cd108d928b7f99f492aa9`. DS1, Class 1, Class 1HF,
Profile 1, and Annex G remain outside this contract.

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
