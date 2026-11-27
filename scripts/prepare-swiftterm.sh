#!/usr/bin/env bash
# Use the published pre-2.0 API; record the actual resolved commit and notices.
set -euo pipefail
cd "$(dirname "$0")/.."
target=Vendor/SwiftTerm
mkdir -p Vendor RuntimeAssets
if [[ ! -e "$target" ]]; then
    git clone --depth 1 --single-branch --branch v1.19.0 https://github.com/migueldeicaza/SwiftTerm.git "$target"
fi
test "$(git -C "$target" remote get-url origin)" = https://github.com/migueldeicaza/SwiftTerm.git
test -z "$(git -C "$target" status --porcelain)"
revision=$(git -C "$target" rev-parse HEAD)
[[ "$revision" == 464df52* ]]
test "$(git -C "$target" rev-parse 'refs/tags/v1.19.0^{commit}')" = "$revision"
license="$target/LICENSE"
if [[ ! -f "$license" ]]; then license="$target/LICENSE.txt"; fi
test -f "$license"
cp "$license" RuntimeAssets/SwiftTerm-LICENSE.txt
printf 'source=https://github.com/migueldeicaza/SwiftTerm.git\nversion=v1.19.0\ncommit=%s\n' "$revision" > RuntimeAssets/SwiftTerm-provenance.txt
