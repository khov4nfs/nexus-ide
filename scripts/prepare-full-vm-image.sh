#!/usr/bin/env bash
# Build reviewed local Ubuntu VM import assets on ARM64 Linux. No publication.
set -euo pipefail
test "$(uname -s)" = Linux && test "$(uname -m)" = aarch64
here="$(cd "$(dirname "$0")" && pwd)"
out="$1"
mkdir -p "$out"
test -z "$(ls -A "$out")"
out="$(cd "$out" && pwd)"
tmp="$(mktemp -d)"
image=
cid=
cleanup() {
    if [[ -n "$cid" ]]; then docker rm -f "$cid" >/dev/null || true; fi
    if [[ -n "$image" ]]; then docker image rm "$image" >/dev/null || true; fi
    sudo rm -rf -- "$tmp"
}
trap cleanup EXIT
# Prepare signed Ubuntu packages on the CI host. Host Docker is used only
# to prepare the disk, not to satisfy any of the in-VM smoke assertions.
base=https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release
name=ubuntu-base-24.04.5-base-arm64.tar.gz
digest=a91d5a93010193712d346d761372b7c9db6dfcf093893161c64ca107f05914f2
curl --proto '=https' --tlsv1.2 --fail --location --retry 3 "$base/$name" -o "$tmp/base.tar.gz"
printf '%s  %s\n' "$digest" "$tmp/base.tar.gz" | sha256sum -c -
image=$(docker import --platform linux/arm64 "$tmp/base.tar.gz")
cid=$(docker create --platform linux/arm64 "$image" /bin/sh -eu -c '
export DEBIAN_FRONTEND=noninteractive
printf "#!/bin/sh\nexit 101\n" >/usr/sbin/policy-rc.d
chmod 755 /usr/sbin/policy-rc.d
printf "%s\n" \
 "deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports noble main universe" \
 "deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports noble-updates main universe" \
 "deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports noble-security main universe" >/tmp/nexus-vm.list
apt-get -q -o Dir::Etc::sourcelist=/tmp/nexus-vm.list -o Dir::Etc::sourceparts=- -o APT::Update::Error-Mode=any update
apt-get -q -y --no-install-recommends -o Dir::Etc::sourcelist=/tmp/nexus-vm.list -o Dir::Etc::sourceparts=- install linux-image-generic initramfs-tools docker.io docker-compose-v2 busybox-static kmod iproute2 iptables systemd-sysv curl ca-certificates
printf "%s\n" virtio_mmio virtio_blk virtio_net ext4 >>/etc/initramfs-tools/modules
kernel=$(ls /lib/modules | sort -V | tail -n 1)
test -n "$kernel"
update-initramfs -u -k "$kernel"
test -s "/boot/vmlinuz-$kernel"
test -s "/boot/initrd.img-$kernel"
cp "/boot/vmlinuz-$kernel" /tmp/nexus-kernel
cp "/boot/initrd.img-$kernel" /tmp/nexus-initrd
chmod 755 /usr/local/sbin/nexus-vm-smoke
# No SSH or remote login. Console acceptance uses the explicit smoke boot.
dpkg-query -W -f="\${Package}\t\${Version}\n" > /tmp/nexus-package-inventory.tsv
apt-get clean
rm -rf /var/lib/apt/lists/*
rm -f /usr/sbin/policy-rc.d /tmp/nexus-vm.list
')
docker cp "$here/vm-tci-guest-smoke.sh" "$cid:/usr/local/sbin/nexus-vm-smoke"
docker start -a "$cid" | tee "$out/preparation.txt"
test "$(docker inspect --format '{{.State.ExitCode}}' "$cid")" = 0
docker cp "$cid:/tmp/nexus-kernel" "$tmp/kernel"
docker cp "$cid:/tmp/nexus-initrd" "$tmp/initrd"
chmod a+r "$tmp/kernel" "$tmp/initrd"
docker cp "$cid:/tmp/nexus-package-inventory.tsv" "$out/packages.tsv"
mkdir "$tmp/root"
docker export "$cid" | sudo tar --numeric-owner -xf - -C "$tmp/root"
sudo rm -f "$tmp/root/tmp/nexus-kernel" "$tmp/root/tmp/nexus-initrd"
truncate -s 4G "$tmp/root.ext4"
sudo mke2fs -q -F -t ext4 -d "$tmp/root" "$tmp/root.ext4"
sudo chown "$(id -u):$(id -g)" "$tmp/root.ext4"
python3 - "$tmp/kernel" "$out/kernel" <<'PY'
import gzip, pathlib, shutil, sys
source, target = map(pathlib.Path, sys.argv[1:])
with source.open("rb") as f:
    head = f.read(64)
if head[56:60] == b"ARMd":
    shutil.copyfile(source, target)
elif head[:2] == b"\x1f\x8b":
    with gzip.open(source, "rb") as f, target.open("xb") as out:
        total = 0
        while chunk := f.read(1024 * 1024):
            total += len(chunk)
            if total > 128 * 1024 * 1024:
                raise ValueError("kernel exceeds app import limit")
            out.write(chunk)
else:
    raise ValueError("expected raw or gzip ARM64 Image")
with target.open("rb") as f:
    f.seek(56)
    assert f.read(4) == b"ARMd"
PY
cp "$tmp/initrd" "$out/initrd"
mv "$tmp/root.ext4" "$out/disk"
test "$(stat -c %s "$out/initrd")" -le 536870912
(cd "$out" && sha256sum kernel initrd disk packages.tsv > SHA256SUMS)

