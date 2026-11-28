"""Synthetic parser regressions; not a substitute for the actual VM boot."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("verify", Path(__file__).with_name("vm-tci-verify-evidence.py"))
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)

class Tests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name)
        common = ["NET_CONFIG_OK", "CONTAINERD_START", "CONTAINERD_READY",
                  "DOCKER_API_READY", "DOCKER_DAEMON_OK"]
        self.restricted = ["PERSIST_PREVIOUS:0", "KERNEL:6.8.0-151-generic",
            "CGROUP2_OK", "RESTRICTED_EGRESS_BLOCKED", *common, "DOCKER_RUN_OK",
            "COMPOSE_OK", "COMPOSE_NETWORK_OK", "FORWARD_READY", "FORWARD_OK",
            "CONTAINERD_STOPPED", "PERSIST_SAVED:1", "CLEAN_SHUTDOWN"]
        self.internet = ["PERSIST_PREVIOUS:0", "KERNEL:6.8.0-151-generic", "CGROUP2_OK",
            "INTERNET_TLS_OK", *common, "REGISTRY_DIGEST:busybox@sha256:"+"a"*64,
            "REGISTRY_PULL_OK", "DOCKER_RUN_OK", "COMPOSE_OK", "CONTAINERD_STOPPED",
            "PERSIST_SAVED:1", "CLEAN_SHUTDOWN"]
        self.write()
        self.http = self.out / "forward-http.json"
        self.http.write_text(json.dumps({"ok": True, "status": 200, "body": "NEXUS_VM_HTTP_42"}))

    def write(self, decorated=True):
        for filename, markers in (("serial.txt", self.restricted), ("internet-serial.txt", self.internet)):
            lines = []
            for marker in markers:
                line = "NEXUS_VM_" + marker
                if decorated and marker in ("COMPOSE_OK", "CONTAINERD_STOPPED"):
                    line = "\x1b[?25h" + line
                lines.append(line)
            (self.out / filename).write_bytes(("\r\n".join(lines)+"\r\n").encode())

    def test_observed_compose_ansi_and_plain_logs(self):
        for decorated in (True, False):
            self.write(decorated)
            report = v.verify(self.out, "pin")
            self.assertTrue(report["ok"])
            self.assertTrue(report["loopbackForwardVerified"])
            self.assertFalse(report["physicalDeviceVerified"])
            self.assertFalse(report["productionReady"])

    def test_missing_duplicate_and_out_of_order_markers_rejected(self):
        original = self.restricted[:]
        for markers in ([m for m in original if m != "COMPOSE_OK"],
                        original + ["COMPOSE_OK"],
                        [*original[:8], original[9], original[8], *original[10:]]):
            self.restricted = markers
            self.write()
            with self.assertRaises(ValueError):
                v.verify(self.out, "pin")

    def test_no_substring_or_internal_escape_matching(self):
        for prefix in ("log: ", "\x1b[31m"):
            self.write()
            p = self.out / "serial.txt"
            p.write_bytes(p.read_bytes().replace(b"\x1b[?25hNEXUS_VM_COMPOSE_OK", prefix.encode()+b"NEXUS_VM_COMPOSE_OK"))
            with self.assertRaises(ValueError):
                v.verify(self.out, "pin")
        self.write()
        p = self.out / "serial.txt"
        p.write_bytes(p.read_bytes().replace(b"NEXUS_VM_COMPOSE_OK", b"NEXUS_VM_COMPOSE_\x1b[?25hOK"))
        with self.assertRaises(ValueError):
            v.verify(self.out, "pin")

    def test_failure_marker_and_wrong_http_rejected(self):
        self.internet.append("FAIL:1")
        self.write()
        with self.assertRaises(AssertionError):
            v.verify(self.out, "pin")
        self.internet.pop()
        self.write()
        self.http.write_text(json.dumps({"ok": True, "status": 200, "body": "wrong"}))
        with self.assertRaises(AssertionError):
            v.verify(self.out, "pin")

if __name__ == "__main__":
    unittest.main()
