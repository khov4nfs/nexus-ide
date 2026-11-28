#!/usr/bin/env bash
# CI-only Linux ARM64 host prerequisite for a future iOS full-system backend.
# Never stage the VM disk, QEMU executable or Docker into RuntimeAssets.
set -euo pipefail
test "$(uname -s)" = Linux
test "$(uname -m)" = aarch64
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
pin=ff3419cbacdc9ad0715c716afeed65bb21a2bbbc
git clone --depth 1 --branch v10.0.2 https://github.com/qemu/qemu.git "$tmp/qemu"
test "$(git -C "$tmp/qemu" rev-parse HEAD)" = "$pin"
mkdir "$tmp/qemu-build"
(
    cd "$tmp/qemu-build"
    "$tmp/qemu/configure" --target-list=aarch64-softmmu \
        --without-default-features --enable-tcg --enable-tcg-interpreter \
        --disable-kvm --disable-hvf --disable-plugins --disable-docs \
        --disable-werror --enable-fdt=system
    grep -Eq '^#define CONFIG_TCG_INTERPRETER( 1)?$' config-host.h
    ! grep -Eq '^#define CONFIG_(KVM|HVF)( 1)?$' config-host.h
    ninja -j "$(nproc)" qemu-system-aarch64
)
qemu="$tmp/qemu-build/qemu-system-aarch64"
"$qemu" --version | tee "$out/qemu-version.txt"

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
apt-get -q -y --no-install-recommends -o Dir::Etc::sourcelist=/tmp/nexus-vm.list -o Dir::Etc::sourceparts=- install linux-image-generic initramfs-tools docker.io docker-compose-v2 busybox-static kmod
printf "%s\n" virtio_mmio virtio_blk ext4 >>/etc/initramfs-tools/modules
kernel=$(ls /lib/modules | sort -V | tail -n 1)
test -n "$kernel"
update-initramfs -u -k "$kernel"
test -s "/boot/vmlinuz-$kernel"
test -s "/boot/initrd.img-$kernel"
cp "/boot/vmlinuz-$kernel" /tmp/nexus-kernel
cp "/boot/initrd.img-$kernel" /tmp/nexus-initrd
chmod 755 /usr/local/sbin/nexus-vm-smoke
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
mkdir "$tmp/root"
docker export "$cid" | sudo tar --numeric-owner -xf - -C "$tmp/root"
sudo rm -f "$tmp/root/tmp/nexus-kernel" "$tmp/root/tmp/nexus-initrd"
truncate -s 4G "$tmp/root.ext4"
sudo mke2fs -q -F -t ext4 -d "$tmp/root" "$tmp/root.ext4"
sudo chown "$(id -u):$(id -g)" "$tmp/root.ext4"
sha256sum "$tmp/kernel" "$tmp/initrd" >"$out/boot-inputs.sha256"

# TCI, not KVM/HVF, no external networking, no host shared filesystem.
# QEMU exit and all ordered guest markers are required.
timeout --signal=TERM --kill-after=20s 45m "$qemu" \
    -machine virt,gic-version=3 -cpu cortex-a57 -accel tcg,thread=single \
    -smp 1 -m 2048 -nodefaults -display none -monitor none -serial stdio \
    -nic none -no-reboot -kernel "$tmp/kernel" -initrd "$tmp/initrd" \
    -append 'root=/dev/vda rw rootwait console=ttyAMA0 init=/usr/local/sbin/nexus-vm-smoke' \
    -drive "if=none,file=$tmp/root.ext4,format=raw,id=root" \
    -device virtio-blk-device,drive=root </dev/null 2>&1 | tee "$out/serial.txt"
python3 - "$out" "$pin" <<'PY'
import json, re, sys
from pathlib import Path
out = Path(sys.argv[1])
lines = (out / "serial.txt").read_text(errors="replace").replace("\r", "").splitlines()
kernel = [line for line in lines if re.fullmatch(r"NEXUS_VM_KERNEL:[0-9A-Za-z._+-]+", line)]
markers = ["NEXUS_VM_CGROUP2_OK", "NEXUS_VM_DOCKER_DAEMON_OK",
           "NEXUS_VM_DOCKER_RUN_OK", "NEXUS_VM_COMPOSE_OK", "NEXUS_VM_CLEAN_SHUTDOWN"]
assert len(kernel) == 1 and not any("NEXUS_VM_FAIL:" in line for line in lines)
assert all(lines.count(marker) == 1 for marker in markers)
positions = [lines.index(kernel[0]), *(lines.index(marker) for marker in markers)]
assert positions == sorted(positions)
report = {"ok": True, "scope": "Linux ARM64 CI host QEMU TCI, offline vfs container smoke",
          "qemuCommit": sys.argv[2], "kernel": kernel[0].split(":", 1)[1],
          "tciBuildVerified": True, "linuxKernelBootVerified": True,
          "dockerEngineInHostTCIVerified": True, "composeInHostTCIVerified": True,
          "cleanShutdown": True, "iosBackendImplemented": False,
          "physicalDeviceVerified": False, "productionReady": False}
(out / "vm-tci-host-evidence.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
PY
