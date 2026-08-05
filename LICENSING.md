# Licensing and provenance policy

## Scope of the root Apache-2.0 licence

The root `LICENSE` covers Emuella-authored software, documentation, schemas,
manifests, recipes, and generated test assets unless a narrower file- or
directory-level notice says otherwise. It does not purport to relicense any
third-party asset obtained through a manifest.

The tracked source tree is designed to be an Apache-2.0 distribution. External
assets are materialized into an ignored cache or artifact directory and retain
their original terms. A materialized cache is therefore a mixed-license
collection even though the catalogue software remains Apache-2.0.

## Rights fields

Each external pack records a human-reviewed status for these distinct uses:

- access;
- redistribution;
- modification;
- commercial use;
- publication of derived assets;
- publication of benchmark results;
- machine-learning training; and
- redistribution of learned weights.

The allowed values are `permitted`, `prohibited`, `conditional`, and
`unknown`. `unknown` is not permission. The evidence URL and review date are
part of the record, because upstream terms can change.

The summaries are operational guardrails, not replacements for the underlying
licences or legal advice. Preserve every upstream licence and attribution file
with a downloaded pack.

## Code and test-data boundary

Running an independently written codec against a corpus does not ordinarily
incorporate the corpus into the codec. Keep that separation explicit:

- do not commit third-party inputs or golden outputs to the codec repositories;
- do not embed image bytes or corpus-derived creative content into Rust source;
- keep transformed or recompressed assets with their source pack lineage;
- publish factual hashes, dimensions, error measurements, and timings without
  embedding source pixels; and
- review any shipped table, heuristic, model, or weight learned from a corpus
  as a separate artifact.

The JPEG XL and JPEG 2000 repositories should pin pack identities and hashes,
not pack bytes. Their ordinary unit tests remain project-authored and
self-contained.

## Contributions

A contributor adding project-authored imagery represents that they have the
right to license it under Apache-2.0. Employment, commissioned-work, privacy,
publicity, trademark, and recognizable-person issues must be resolved before
acceptance.

Third-party assets are manifest-only unless redistribution has been positively
reviewed. Public availability, a downloadable URL, or an upstream Git
repository is not evidence of redistribution permission.

## ISO and other restricted attachments

ISO electronic inserts and other purpose-restricted conformance assets are not
vendored or republished here. A manifest may identify an authoritative source,
standard edition, filename, and expected digest. Users obtain the material
under the upstream terms and provide it to the verifier themselves.

## JPEG AI and learned artifacts

Permission to benchmark or perform conformance testing is not permission to
train. Training datasets, validation datasets, conformance assets, and learned
weights are separate pack kinds. `ml_training = "unknown"` or
`weights_redistribution = "unknown"` prevents a pack from entering a training
or model-release suite.
