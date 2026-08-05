# Adding a corpus

## 1. Classify the purpose

Choose one or more precise purposes: `conformance`, `interoperability`,
`regression`, `robustness`, `quality-benchmark`, `throughput-benchmark`,
`training`, or `validation`. Do not label an implementation regression corpus
as standards conformance.

## 2. Establish provenance

Record the authoritative project or publisher, exact edition or commit,
original filename, landing page, direct source when stable, retrieval date,
and upstream attribution. Prefer a publisher's page over a third-party mirror.

## 3. Review rights independently

For every rights field, record `permitted`, `prohibited`, `conditional`, or
`unknown`, plus an evidence URL and concise note. Check click-through terms and
custom notices inside the archive. Do not infer an image licence from the
licence of a loader, web page, codec, or GitHub repository.

Machine-learning training and weight redistribution always receive separate
answers. They never inherit permission from benchmarking.

## 4. Decide the storage mode

- Use `generated` for deterministic Emuella-authored assets.
- Use `external` when users acquire authoritative upstream bytes.
- Use `derived` only after the source and transformation rights are reviewed.

External packs with conditional or unknown redistribution are not vendored.

## 5. Lock integrity

Before changing `review_state` to `locked`:

1. preserve the upstream archive unchanged;
2. record its SHA-256;
3. extract it without modifying contents;
4. retain embedded licence and attribution files;
5. list every selected asset with size and SHA-256; and
6. run `emuella-corpus check` and `verify`.

For a complete external tree, generate the inventory with:

```sh
cargo run -p emuella-corpus -- inventory PACK_ID \
  --root MATERIALIZED_ROOT --output inventories/PACK_VERSION.toml
```

Record the reported tree digest in `materialization.expected_tree_sha256` and
reference the inventory from `asset_inventory`. Keep the unchanged upstream
archive at the materialization root so its digest and extracted tree are
verified together.

If upstream silently replaces an archive, create a new pack version. Do not
change the digest of a released version.

## 6. Add suites conservatively

Release-gating suites use locked packs only. Large, restricted, or optional
packs should not enter `ci-fast`. A suite declares whether absent data is a
skip or failure.
