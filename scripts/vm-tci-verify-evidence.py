import json, re, sys
from pathlib import Path

# Compose writes cursor visibility control sequences even on the guest
# serial console. Strip only those exact sequences at line boundaries.
# Keep marker content, counts and ordering strict; do not search substrings.
CURSOR_EDGES = re.compile(r"^(?:\x1b\[\?25[hl])+|(?:\x1b\[\?25[hl])+$")
def serial_lines(path):
    return [CURSOR_EDGES.sub("", line) for line in path.read_text(errors="replace").replace("\r", "").split("\n")]

def require_markers(lines, markers):
    for marker in markers:
        if lines.count(marker) != 1:
            raise ValueError(f"Expected exactly one {marker}, got {lines.count(marker)}")
    positions = [lines.index(marker) for marker in markers]
    if positions != sorted(positions):
        raise ValueError("Acceptance markers out of order")

def verify(out, pin):
    lines = serial_lines(out / "serial.txt")
    kernel = [line for line in lines if re.fullmatch(r"NEXUS_VM_KERNEL:[0-9A-Za-z._+-]+", line)]
    markers = ["NEXUS_VM_CGROUP2_OK", "NEXUS_VM_RESTRICTED_EGRESS_BLOCKED", "NEXUS_VM_NET_CONFIG_OK", "NEXUS_VM_CONTAINERD_START", "NEXUS_VM_CONTAINERD_READY", "NEXUS_VM_DOCKER_API_READY", "NEXUS_VM_DOCKER_DAEMON_OK",
               "NEXUS_VM_DOCKER_RUN_OK", "NEXUS_VM_COMPOSE_OK", "NEXUS_VM_COMPOSE_NETWORK_OK",
               "NEXUS_VM_FORWARD_READY", "NEXUS_VM_FORWARD_OK", "NEXUS_VM_CONTAINERD_STOPPED", "NEXUS_VM_CLEAN_SHUTDOWN"]
    assert len(kernel) == 1 and not any("NEXUS_VM_FAIL:" in line for line in lines)
    require_markers(lines, [kernel[0], *markers])
    assert lines.count("NEXUS_VM_PERSIST_PREVIOUS:0")==1 and lines.count("NEXUS_VM_PERSIST_SAVED:1")==1
    http = json.loads((out / "forward-http.json").read_text())
    assert http == {"ok": True, "status": 200, "body": "NEXUS_VM_HTTP_42"}
    internet = serial_lines(out / "internet-serial.txt")
    assert not any("NEXUS_VM_FAIL:" in line for line in internet)
    internet_markers = ("NEXUS_VM_INTERNET_TLS_OK", "NEXUS_VM_NET_CONFIG_OK", "NEXUS_VM_CONTAINERD_START",
        "NEXUS_VM_CONTAINERD_READY", "NEXUS_VM_DOCKER_API_READY", "NEXUS_VM_DOCKER_DAEMON_OK", "NEXUS_VM_REGISTRY_PULL_OK",
        "NEXUS_VM_DOCKER_RUN_OK", "NEXUS_VM_COMPOSE_OK", "NEXUS_VM_CONTAINERD_STOPPED", "NEXUS_VM_CLEAN_SHUTDOWN")
    require_markers(internet, internet_markers)
    registry = [line.split(":", 1)[1] for line in internet if re.fullmatch(r"NEXUS_VM_REGISTRY_DIGEST:(?:docker.io/library/)?busybox@sha256:[0-9a-f]{64}", line)]
    assert len(registry) == 1
    report = {"ok": True, "internetTLSVerified": True, "registryPullVerified": True, "restrictedRegistryEgressBlocked": True, "registryDigest": registry[0], "scope": "Linux ARM64 CI host QEMU TCI, restricted SLIRP and vfs container smoke",
              "qemuCommit": pin, "kernel": kernel[0].split(":", 1)[1],
              "tciBuildVerified": True, "linuxKernelBootVerified": True,
              "dockerEngineInHostTCIVerified": True, "composeInHostTCIVerified": True,
              "composeNetworkVerified": True, "loopbackForwardVerified": True,
              "cleanShutdown": True, "iosBackendVerified": False,
              "physicalDeviceVerified": False, "productionReady": False}
    return report

if __name__ == "__main__":
    out = Path(sys.argv[1])
    report = verify(out, sys.argv[2])
    (out / "vm-tci-host-evidence.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
