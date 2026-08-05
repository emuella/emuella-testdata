# Contributing

Contributions to catalogue software, documentation, schemas, recipes, and
project-authored generated fixtures are submitted under Apache-2.0 unless
explicitly marked otherwise.

## Adding a corpus

Follow [Adding a corpus](docs/ADDING_A_CORPUS.md). A new external manifest must
identify exact provenance, upstream terms, intended uses, and every unknown or
conditional right. Do not add image bytes while a rights field is unknown.

## Adding generated imagery

Generated imagery must be reproducible from reviewed source code or recipes.
Avoid unpinned fonts, color-management defaults, random seeds, timestamps, and
tool versions. Record canonical pixel semantics and output-file encoding
separately.

## Review expectations

Changes should pass formatting, compilation, tests, catalogue validation,
generated-tree comparison, and dependency policy checks listed in `README.md`.
Reviewers should inspect licence scope and provenance independently of whether
the files happen to pass the technical checks.
