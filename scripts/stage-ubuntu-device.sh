#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
  "") source=Vendor/litter-ish/build/ubuntu-fs ;;
  --agy) source=Vendor/litter-ish/build/agy-fs ;;
  *) echo "Usage: $0 [--agy]" >&2; exit 2 ;;
esac
[[ $# -le 1 ]]
test -d "$source/data"
test -f "$source/meta.db"
test -f RuntimeAssets/lib/libnexus_guest_bridge.a
test ! -e RuntimeAssets/ubuntu-fs || { echo "Move existing RuntimeAssets/ubuntu-fs aside first" >&2; exit 1; }
if [[ "${1:-}" == --agy ]]; then
  test -f "$source/agy.sha256"
  expected=$(cat "$source/agy.sha256")
  [[ "$expected" =~ ^[a-f0-9]{64}$ ]]
  printf '%s  %s\n' "$expected" "$source/data/opt/nexus/agy" | shasum -a 256 -c -
else
  test ! -e "$source/agy.sha256"
  test ! -e "$source/data/opt/nexus/agy"
fi
# Source must be fresh and stopped. No copying an active fakefs database.
test ! -e "$source/meta.db-wal"
test ! -e "$source/meta.db-shm"
stage=$(mktemp -d RuntimeAssets/ubuntu-stage.XXXXXX)
trap 'rm -rf "$stage"' EXIT
cp -a "$source" "$stage/ubuntu-fs"
mv "$stage/ubuntu-fs" RuntimeAssets/ubuntu-fs
xcodegen generate --spec project-glibc.yml
echo "Generated local Ubuntu Lab target. Do not distribute AGY-containing builds."
