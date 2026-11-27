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

if __name__ == "__main__":
    unittest.main()
