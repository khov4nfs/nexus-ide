#!/usr/bin/env bash
# CI only. The app never runs Docker or depends on this prepared test filesystem.
set -euo pipefail
out="$1"
mkdir -p "$out"
test -z "$(ls -A "$out")"
test "$(uname -m)" = aarch64
base=https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release
name=ubuntu-base-24.04.5-base-arm64.tar.gz
digest=a91d5a93010193712d346d761372b7c9db6dfcf093893161c64ca107f05914f2
scratch=$(mktemp -d)
image=
cid=
cleanup() {
    if [[ -n "$cid" ]]; then docker rm -f "$cid" >/dev/null || true; fi
    if [[ -n "$image" ]]; then docker image rm "$image" >/dev/null || true; fi
    rm -rf "$scratch"
}
trap cleanup EXIT
curl --proto '=https' --tlsv1.2 --fail --location --retry 3 "$base/$name" -o "$scratch/$name"
printf '%s  %s\n' "$digest" "$scratch/$name" | sha256sum -c -
image=$(docker import --platform linux/arm64 "$scratch/$name")
cid=$(docker create --platform linux/arm64 "$image" /bin/sh -eu -c '
export DEBIAN_FRONTEND=noninteractive
test "$(uname -m)" = aarch64
test ! -e /usr/bin/pylsp
printf "%s\n" "deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports noble main universe" >/tmp/nexus-pylsp.list
apt-get -q -o Dir::Etc::sourcelist=/tmp/nexus-pylsp.list -o Dir::Etc::sourceparts=- -o Acquire::Retries=3 -o APT::Update::Error-Mode=any update
apt-get -q -y --no-install-recommends -o Dir::Etc::sourcelist=/tmp/nexus-pylsp.list -o Dir::Etc::sourceparts=- install python3-pylsp=1.10.0-1 python3-pyflakes
test "$(dpkg-query -W -f="\${Version}" python3-pylsp)" = 1.10.0-1
/usr/bin/python3 -c "import pylsp, pyflakes; from importlib.metadata import version; assert version(\"python-lsp-server\") == \"1.10.0\""
/usr/bin/python3 /tmp/nexus-pylsp-native-probe.py
rm -f /tmp/nexus-pylsp-native-probe.py
apt-get clean
rm -rf /var/lib/apt/lists/*
rm -f /tmp/nexus-pylsp.list
')
docker cp "$(dirname "$0")/ci-pylsp-native-probe.py" "$cid:/tmp/nexus-pylsp-native-probe.py"
docker start -a "$cid"
test "$(docker inspect --format '{{.State.ExitCode}}' "$cid")" = 0
docker export "$cid" | gzip -n > "$out/pylsp-rootfs.tar.gz"
test -s "$out/pylsp-rootfs.tar.gz"
(cd "$out" && sha256sum pylsp-rootfs.tar.gz > SHA256SUMS)
