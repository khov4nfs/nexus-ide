#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
  "") source=Vendor/litter-ish/build/ubuntu-fs ;;
  --agy) source=Vendor/litter-ish/build/agy-fs ;;
  --workbench) source=Vendor/litter-ish/build/workbench-fs ;;
  *) echo "Usage: $0 [--agy|--workbench]" >&2; exit 2 ;;
esac
[[ $# -le 1 ]]
test -d "$source/data"
test -f "$source/meta.db"
test -f RuntimeAssets/lib/libnexus_guest_bridge.a
test ! -e RuntimeAssets/ubuntu-fs || { echo "Move existing RuntimeAssets/ubuntu-fs aside first" >&2; exit 1; }
if [[ "${1:-}" == --agy ]] || [[ "${1:-}" == --workbench && -f "$source/agy.sha256" ]]; then
  test -f "$source/agy.sha256"
  expected=$(cat "$source/agy.sha256")
  [[ "$expected" =~ ^[a-f0-9]{64}$ ]]
  printf '%s  %s\n' "$expected" "$source/data/opt/nexus/agy" | shasum -a 256 -c -
else
  test ! -e "$source/agy.sha256"
  test -f "$source/data/opt/nexus/agy"
  printf 'NEXUS_AGY_IMPORT_SLOT_V1\n' | cmp - "$source/data/opt/nexus/agy"
fi
if [[ "${1:-}" == --workbench ]]; then
  test "$(cat "$source/workbench.sha256")" = 36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc
  printf 'NEXUS_VSIX_IMPORT_SLOT_V1\n' | cmp - "$source/data/opt/nexus/extension.vsix"
  for item in launcher node; do
    expected=$(cat "$source/workbench-$item.sha256")
    [[ "$expected" =~ ^[a-f0-9]{64}$ ]]
    if [[ "$item" == launcher ]]; then rel=bin/openvscode-server; else rel=node; fi
    printf '%s  %s\n' "$expected" "$source/data/opt/nexus/workbench/$rel" | shasum -a 256 -c -
  done
else
  test ! -e "$source/workbench.sha256"
fi
# Source must be fresh and stopped. No copying an active fakefs database.
test ! -e "$source/meta.db-wal"
test ! -e "$source/meta.db-shm"
stage=$(mktemp -d RuntimeAssets/ubuntu-stage.XXXXXX)
trap 'rm -rf "$stage"' EXIT
cp -a "$source" "$stage/ubuntu-fs"
mv "$stage/ubuntu-fs" RuntimeAssets/ubuntu-fs
bash scripts/prepare-swiftterm.sh
xcodegen generate --spec project-glibc.yml
echo "Generated local Ubuntu Lab target. Do not distribute AGY-containing builds."
