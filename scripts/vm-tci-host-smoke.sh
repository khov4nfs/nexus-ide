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
python3 - "$out" "$pin" <<'PY'
import json, re, sys
from pathlib import Path
out = Path(sys.argv[1])
lines = (out / "serial.txt").read_text(errors="replace").replace("\r", "").splitlines()
kernel = [line for line in lines if re.fullmatch(r"NEXUS_VM_KERNEL:[0-9A-Za-z._+-]+", line)]
markers = ["NEXUS_VM_CGROUP2_OK", "NEXUS_VM_NET_CONFIG_OK", "NEXUS_VM_DOCKER_DAEMON_OK",
           "NEXUS_VM_DOCKER_RUN_OK", "NEXUS_VM_COMPOSE_OK", "NEXUS_VM_COMPOSE_NETWORK_OK",
           "NEXUS_VM_FORWARD_READY", "NEXUS_VM_FORWARD_OK", "NEXUS_VM_CLEAN_SHUTDOWN"]
assert len(kernel) == 1 and not any("NEXUS_VM_FAIL:" in line for line in lines)
assert all(lines.count(marker) == 1 for marker in markers)
positions = [lines.index(kernel[0]), *(lines.index(marker) for marker in markers)]
assert positions == sorted(positions)
http = json.loads((out / "forward-http.json").read_text())
assert http == {"ok": True, "status": 200, "body": "NEXUS_VM_HTTP_42"}
report = {"ok": True, "scope": "Linux ARM64 CI host QEMU TCI, restricted SLIRP and vfs container smoke",
          "qemuCommit": sys.argv[2], "kernel": kernel[0].split(":", 1)[1],
          "tciBuildVerified": True, "linuxKernelBootVerified": True,
          "dockerEngineInHostTCIVerified": True, "composeInHostTCIVerified": True,
          "composeNetworkVerified": True, "loopbackForwardVerified": True,
          "cleanShutdown": True, "iosBackendVerified": False,
          "physicalDeviceVerified": False, "productionReady": False}
(out / "vm-tci-host-evidence.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
PY
