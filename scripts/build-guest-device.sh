#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
PIN=c8e9dcb954963b0d9f359b3ad9da871f19b28652
UPSTREAM=https://github.com/dnakov/litter-ish.git
ALPINE=3.21.0
FILE=alpine-minirootfs-${ALPINE}-aarch64.tar.gz
URL=https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/$FILE
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo "Requires Apple Silicon macOS + Xcode" >&2; exit 1; }
for tool in git cargo rustup meson ninja xcodegen xcodebuild curl shasum sqlite3 brew; do
  command -v "$tool" >/dev/null || { echo "Missing $tool" >&2; exit 1; }
done
export PATH="$(brew --prefix llvm)/bin:$PATH"
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export IPHONEOS_DEPLOYMENT_TARGET=17.0
mkdir -p Vendor RuntimeAssets/lib
if [[ ! -e Vendor/litter-ish ]]; then
  git clone --no-checkout "$UPSTREAM" Vendor/litter-ish
  git -C Vendor/litter-ish checkout --detach "$PIN"
  git -C Vendor/litter-ish submodule update --init --recursive
fi
[[ "$(git -C Vendor/litter-ish rev-parse HEAD)" == "$PIN" ]] || { echo "Wrong upstream commit; refusing build" >&2; exit 1; }
[[ -z "$(git -C Vendor/litter-ish status --porcelain --untracked-files=no)" ]] || { echo "Upstream has tracked modifications; refusing build" >&2; exit 1; }
rustup target add aarch64-apple-ios aarch64-unknown-linux-musl
mkdir -p Vendor/litter-ish/build
# Verify transfer integrity using the official Alpine checksum over TLS.
# For independently pinned input, set ROOTFS_SHA256 from a reviewed manifest.
checksum="${ROOTFS_SHA256:-}"
if [[ -z "$checksum" ]]; then
  checksum="$(curl --proto '=https' --tlsv1.2 -fsSL "$URL.sha256" | awk 'NR==1{print $1}')"
fi
[[ "$checksum" =~ ^[a-fA-F0-9]{64}$ ]] || { echo "Invalid rootfs digest" >&2; exit 1; }
tarball="$ROOT/Vendor/litter-ish/build/$FILE"
if [[ ! -f "$tarball" ]]; then
  curl --proto '=https' --tlsv1.2 -fsSL "$URL" -o "$tarball.tmp"
  mv "$tarball.tmp" "$tarball"
fi
printf '%s  %s\n' "$checksum" "$tarball" | shasum -a 256 -c -
ALPINE_VERSION="$ALPINE" bash Vendor/litter-ish/embed/build-rootfs.sh
test -f Vendor/litter-ish/build/fs/meta.db
test -d Vendor/litter-ish/build/fs/data
# Stage a fresh copy; never overwrite the device's persistent workspace.
stage="$(mktemp -d "$ROOT/RuntimeAssets/stage.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
cp -a Vendor/litter-ish/build/fs "$stage/fs"
rm -rf RuntimeAssets/fs
mv "$stage/fs" RuntimeAssets/fs
cargo build --manifest-path RuntimeBridge/Cargo.toml --release --target aarch64-apple-ios
cp RuntimeBridge/target/aarch64-apple-ios/release/libnexus_guest_bridge.a RuntimeAssets/lib/
printf 'litter-ish=%s\nalpine=%s\nrootfs_sha256=%s\n' "$PIN" "$ALPINE" "$checksum" > RuntimeAssets/provenance.txt
xcodegen generate --spec project-guest.yml
echo "Generated NexusIDEGuest.xcodeproj. Select signing team and run on physical iPhone."
