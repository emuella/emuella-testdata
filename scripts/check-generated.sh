#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/emuella-generated.XXXXXX")
trap 'rm -rf -- "$temporary_root"' EXIT HUP INT TERM

cd "$repository_root"
cargo run --quiet -p emuella-corpus -- \
  generate common/generated-core --output "$temporary_root/generated-core"
diff -r --no-dereference \
  generated/common/generated-core \
  "$temporary_root/generated-core"
