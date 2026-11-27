#!/usr/bin/env bash
# Independent compatibility rootfs. Never replaces the working Alpine image.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD"
base=https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release
name=ubuntu-base-24.04.5-base-arm64.tar.gz
digest=a91d5a93010193712d346d761372b7c9db6dfcf093893161c64ca107f05914f2
converter="$root/Vendor/litter-ish/build/host-fakefsify/tools/fakefsify"
test -x "$converter" || { echo "Run scripts/build-guest-device.sh first" >&2; exit 1; }
test "$(git -C Vendor/litter-ish rev-parse HEAD)" = c8e9dcb954963b0d9f359b3ad9da871f19b28652
python3 scripts/apply-runtime-patches.py Vendor/litter-ish
mkdir -p "$root/Vendor/litter-ish/build"
cache="$root/Vendor/litter-ish/build/$name"
if [[ ! -f "$cache" ]]; then
    curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location "$base/$name" -o "$cache.tmp"
    mv "$cache.tmp" "$cache"
fi
printf '%s  %s\n' "$digest" "$cache" | shasum -a 256 -c -
stage=$(mktemp -d "$root/Vendor/litter-ish/build/ubuntu-stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
python3 - "$cache" "$stage/slot.tar.gz" <<'PY'
import importlib.util, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location("agy", "scripts/prepare-agy-rootfs.py")
agy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agy)
agy.overlay_import_slot(Path(sys.argv[1]), Path(sys.argv[2]))
PY
"$converter" "$stage/slot.tar.gz" "$stage/fs"
test "$(cat "$stage/fs/data/opt/nexus/agy")" = NEXUS_AGY_IMPORT_SLOT_V1
sqlite3 "$stage/fs/meta.db" 'PRAGMA wal_checkpoint(TRUNCATE);'
test -d "$stage/fs/data"
test -f "$stage/fs/meta.db"
rm -f "$stage/fs/meta.db-shm" "$stage/fs/meta.db-wal"
target="$root/Vendor/litter-ish/build/ubuntu-fs"
test ! -e "$target" || { echo "Refusing to overwrite existing Ubuntu rootfs; move it aside explicitly" >&2; exit 1; }
mv "$stage/fs" "$target"
printf 'source=%s/%s\nsha256=%s\nagy_import_slot=v1\n' "$base" "$name" "$digest" > "$target/provenance.txt"
echo "Prepared separate Ubuntu fakefs at $target/data"
