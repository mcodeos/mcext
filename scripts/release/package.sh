#!/bin/sh
# Make the extension package.
#
# The package is the release artifact, and its name carries the batch
# number: the coordinate of the extension exists only once the artifact
# exists, so the gate reads the number off the bytes that go out — not
# off a copy that happens to sit in the working tree.
#
#     MCC_BUILD_NR=8675 sh scripts/release/package.sh
#
# The batch number is an environment fact, never a path written here.
#
set -eu

: "${MCC_BUILD_NR:?the batch number must be given: MCC_BUILD_NR}"

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
out_dir="$root/build/release/dist"
version=$(node -p "require('$root/package.json').version")

# An earlier package of an earlier batch is an old piece lying in the
# build tree: it is not this extension carrying an old number, and the
# gate must not read it as drift.  One batch, one package.
mkdir -p "$out_dir"
rm -f "$out_dir"/mcode-*.vsix

cd "$root"
npm run package -- --out "$out_dir/mcode-$version.b$MCC_BUILD_NR.vsix"
