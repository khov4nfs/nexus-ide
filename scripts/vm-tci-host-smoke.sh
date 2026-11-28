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
        --disable-werror --enable-fdt=system --enable-slirp
    grep -Eq '^#define CONFIG_TCG_INTERPRETER( 1)?$' config-host.h
    ! grep -Eq '^#define CONFIG_(KVM|HVF)( 1)?$' config-host.h
    ninja -j "$(nproc)" qemu-system-aarch64
)
qemu="$tmp/qemu-build/qemu-system-aarch64"
"$qemu" --version | tee "$out/qemu-version.txt"

# Build exactly the import-compatible image; test only a copy of its mutable disk.
bash "$here/prepare-full-vm-image.sh" "$tmp/import"
cp "$tmp/import/SHA256SUMS" "$out/boot-inputs.sha256"
cp "$tmp/import/packages.tsv" "$out/packages.tsv"
cp "$tmp/import/kernel" "$tmp/kernel"
cp "$tmp/import/initrd" "$tmp/initrd"
cp --sparse=always "$tmp/import/disk" "$tmp/root.ext4"

# Run the exact restricted network topology implemented in the iOS backend.
# The HTTP request originates on the host; only an actual container response
# and guest-side acknowledgement can satisfy the gate.
python3 "$here/vm-tci-network-probe.py" "$out" "$qemu" "$tmp/kernel" "$tmp/initrd" "$tmp/root.ext4"
cp --sparse=always "$tmp/import/disk" "$tmp/internet.ext4"
timeout --signal=TERM --kill-after=20s 45m "$qemu" \
    -machine virt,gic-version=3 -cpu cortex-a57 -accel tcg,thread=single -smp 1 -m 1024 \
    -nodefaults -no-user-config -display none -monitor none -serial stdio -no-reboot \
    -kernel "$tmp/kernel" -initrd "$tmp/initrd" \
    -append 'root=/dev/vda rw rootwait console=ttyAMA0 init=/usr/local/sbin/nexus-vm-smoke nexus.net=2' \
    -drive "if=none,file=$tmp/internet.ext4,format=raw,id=root" -device virtio-blk-device,drive=root \
    -netdev user,id=nexusnet,restrict=off,ipv6=off -device virtio-net-device,netdev=nexusnet \
    </dev/null 2>&1 | tee "$out/internet-serial.txt"
python3 "$here/vm-tci-verify-evidence.py" "$out" "$pin"
