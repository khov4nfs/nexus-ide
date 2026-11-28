import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("acceptance", Path(__file__).with_name("collect-release-acceptance.py"))
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)

class Tests(unittest.TestCase):
    def test_inventory_and_integrity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "pkg"
            pkg.mkdir()
            commit = "a" * 40
            name = f"NexusIDE-ubuntu-{commit[:12]}-unsigned.ipa"
            ipa = pkg / name
            ipa.write_bytes(b"synthetic package bytes; not an IPA")
            build = {"commit": commit, "variant": "ubuntu", "physicalDeviceVerified": False,
                     "file": name, "bytes": ipa.stat().st_size, "sha256": hashlib.sha256(ipa.read_bytes()).hexdigest()}
            (pkg / "build.json").write_text(json.dumps(build))
            for key, (_, required) in a.gates("ubuntu").items():
                (root / f"nexus-{key}.json").write_text(json.dumps(required))
            report = a.collect(root, pkg, commit, "ubuntu")
            self.assertTrue(report["ciAcceptancePassed"])
            self.assertFalse(report["productionReady"])
            self.assertFalse(report["physicalDeviceVerified"])
            self.assertEqual(len(report["checks"]), len(a.gates("ubuntu")))
            with self.assertRaises(ValueError):
                a.collect(root, pkg, "b" * 40, "ubuntu")
            target = root / "nexus-lsp-success.json"
            original = target.read_bytes()
            target.write_text(json.dumps({"ok": 1, "fixtureOnly": True, "shutdownClean": True, "phase": "success"}))
            with self.assertRaises(ValueError):
                a.collect(root, pkg, commit, "ubuntu")
            target.unlink()
            with self.assertRaises(ValueError):
                a.collect(root, pkg, commit, "ubuntu")
            target.write_bytes(original)
            ipa.write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                a.collect(root, pkg, commit, "ubuntu")


    def test_full_vm_requires_host_and_exact_build_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "pkg"
            pkg.mkdir()
            commit = "a" * 40
            name = f"NexusIDE-workbench-{commit[:12]}-unsigned.ipa"
            ipa = pkg / name
            ipa.write_bytes(b"synthetic VM gate test only")
            digest = hashlib.sha256(ipa.read_bytes()).hexdigest()
            build = {"commit": commit, "variant": "workbench", "physicalDeviceVerified": False,
                     "file": name, "bytes": ipa.stat().st_size, "sha256": digest,
                     "fullVMIncluded": True, "fullVMSourceInventorySHA256": "c" * 64}
            (pkg / "build.json").write_text(json.dumps(build))
            vsix = pkg / "nexus-real-pylsp.vsix"
            vsix.write_bytes(b"synthetic test VSIX")
            for key, (_, required) in a.gates("workbench").items():
                fields = dict(required)
                if key == "workbench-real-lsp":
                    fields["vsixSha256"] = hashlib.sha256(vsix.read_bytes()).hexdigest()
                (root / f"nexus-{key}.json").write_text(json.dumps(fields))
            with self.assertRaises(ValueError):
                a.collect(root, pkg, commit, "workbench")
            for key in ("full-vm-host", "full-vm-build"):
                required = dict(a.gates("workbench", True)[key][1])
                if key == "full-vm-build":
                    required.update(commit=commit, ipaSha256=digest, sourceInventorySHA256="d" * 64)
                (root / f"nexus-{key}.json").write_text(json.dumps(required))
            with self.assertRaises(ValueError):
                a.collect(root, pkg, commit, "workbench")
            path = root / "nexus-full-vm-build.json"
            report = json.loads(path.read_text())
            report["sourceInventorySHA256"] = "c" * 64
            path.write_text(json.dumps(report))
            accepted = a.collect(root, pkg, commit, "workbench")
            self.assertTrue(accepted["fullVMIncluded"])
            self.assertFalse(accepted["dockerEngineVerified"])
            self.assertFalse(accepted["productionReady"])
            (root / "nexus-full-vm-host.json").unlink()
            with self.assertRaises(ValueError):
                a.collect(root, pkg, commit, "workbench")

if __name__ == "__main__":
    unittest.main()
