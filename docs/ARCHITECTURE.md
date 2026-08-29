# Architecture

## Goals

The catalogue provides reproducible test inputs without making mixed-license
assets part of an Apache-2.0 codec distribution. It distinguishes four
technical purposes that should not be conflated:

- **conformance** checks behavior defined by a standards conformance part;
- **regression and interoperability** preserve behavior across implementations
  and known real-world files;
- **robustness** exercises invalid, truncated, hostile, or unusual input; and
- **benchmarking** measures throughput, memory, compression ratio, and quality.

A file can appear in more than one suite only when its manifest authorizes and
describes every use.

## Three test layers

### Layer 1: self-contained unit fixtures

Layer 1 is always available after cloning a codec repository. It uses
project-authored bytes and mathematical cases, is fast, needs no network, and
must remain distributable under the codec repository's Apache-2.0 licence.

`common/generated-core` supplements codec-local fixtures with reusable source
imagery. The generator is deterministic, and the manifest records every
output's size, media type, sample semantics, and digest.

### Layer 2: optional corpus integration

Layer 2 takes a pack ID and an explicit materialized root. Packs are immutable
once locked. A new upstream archive, selection, conversion recipe, or expected
tree receives a new version.

The catalogue never assumes that access implies redistribution. A pack whose
terms require acknowledgement or manual acquisition is resolved by the user,
then verified locally.

Rendered-pixel comparisons are a sibling contract to native component
comparisons. The initial Annex G contract binds a locked JP2 input and TIFF
reference by inventory path, describes one full-frame 8-bit sRGB result, and
sets one inclusive aggregate peak-error limit. It does not select a native
component, reduction or region, prescribe interpolation, or record decoded
pixels, decoded digests, per-pixel results or mean-squared error. A dedicated
codec worker or runner must execute this plan later against an already
materialised and verified pack; the catalogue neither acquires missing data
nor supplies a decoder fallback.

The JSON Schema owns portable per-field shape: required fields, types, fixed
rendering values, numeric bounds, non-blank authority text and safe lexical
path forms. Catalogue validation remains authoritative for selected-pack and
locked-inventory membership, input/reference inequality, and uniqueness of
case IDs, inputs and references across records. Standard JSON Schema cannot
portably express those inventory lookups or relational uniqueness rules. The
current `.jp2` and `.tif` lexical patterns also make a valid case's two paths
different by construction, but the catalogue retains the explicit semantic
inequality check as a defence if the admitted formats later broaden.

### Layer 3: pinned CI qualification

A Layer 3 result is identified by all of:

- codec repository and commit;
- catalogue repository and commit;
- suite ID and revision;
- every pack ID, version, and tree digest;
- harness and toolchain versions;
- target, CPU features, thread count, and relevant environment; and
- result schema revision.

Public CI must not accept a licence or expose a restricted asset on behalf of
unrelated users. Pre-provisioned caches are preferred for purpose-restricted
material.

## Pack lifecycle

Pack review states are:

- `planned`: useful source identified; integrity or terms work remains;
- `reviewed`: provenance and rights were reviewed, but bytes may not yet be
  locked;
- `locked`: authoritative source, archive digest, and materialized tree are
  immutable and reproducible;
- `withdrawn`: no longer offered for new runs; historical identities remain.

Only locked packs belong in release-qualification suites. Development suites
may name reviewed or planned packs when marked non-gating.

## Storage

Tracked Git content is limited to small project-authored fixtures. External
pack bytes live under `EMUELLA_TESTDATA_CACHE`, defaulting to `.cache` when a user
chooses to create it. Published derived packs should use content-addressed
release assets or object storage rather than Git history.

The verifier rejects absolute or parent-traversing asset paths. A materialized
tree must retain upstream licence and provenance files. A locked external pack
records every regular file in a separate inventory and rejects symlinks or
other special filesystem entries.

The canonical tree digest is SHA-256 over the inventory sorted by path. Each
record is encoded as lowercase file SHA-256, a tab, its decimal byte length, a
tab, its forward-slash relative path, and a newline. Media type and descriptive
semantics are intentionally excluded from the digest, so improving catalogue
metadata does not change the identity of an unchanged byte tree. Additional,
missing, renamed, or modified files change the digest.

## Stable identities

Pack IDs are path-like and codec-neutral where possible:

```text
common/generated-core
common/cid22-reference
jpeg-xl/conformance
jpeg-2000/conformance
jpeg-2000/openjpeg-regressions
jpeg-ai/validation/example
```

Storage locations and URLs may change without changing an ID. Semantic content
or licensing changes require a new pack version.
