import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("device", Path(__file__).with_name("verify-device-acceptance.py"))
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)

class TestDeviceAcceptance(unittest.TestCase):
    def test_manual_scope_and_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "pkg"
            pkg.mkdir()
            commit = "a" * 40
            name = f"NexusIDE-ubuntu-{commit[:12]}-unsigned.ipa"
            ipa = pkg / name
            ipa.write_bytes(b"synthetic test bytes; not an IPA")
            (pkg / "build.json").write_text(json.dumps({"commit": commit, "variant": "ubuntu", "file": name,
                "bytes": ipa.stat().st_size, "sha256": d.sha_file(ipa), "physicalDeviceVerified": False}))
            for key, (_, fields) in d.ci.gates("ubuntu").items():
                (root / f"nexus-{key}.json").write_text(json.dumps(fields))
            (pkg / "acceptance.json").write_text(json.dumps(d.ci.collect(root,pkg,commit,"ubuntu")))
            ident = d.package_identity(pkg)
            record = d.template(ident)
            d.verify_release_template(ident, record)
            self.assertEqual(d.package_identity(pkg, commit, "ubuntu"), ident)
            with self.assertRaises(ValueError):
                d.package_identity(pkg, "b" * 40, "ubuntu")
            with self.assertRaises(ValueError):
                d.package_identity(pkg, commit, "workbench")
            bad = d.template(ident)
            bad["checks"]["agy_google_auth_real"]["passed"] = 0
            with self.assertRaises(ValueError):
                d.verify_release_template(ident, bad)
            manifest = pkg / "acceptance.json"
            original = manifest.read_bytes()
            tampered = json.loads(original)
            tampered["checks"][0]["scope"] = "physical device"
            manifest.write_text(json.dumps(tampered))
            with self.assertRaises(ValueError):
                d.package_identity(pkg)
            manifest.write_bytes(original)
            with self.assertRaises(ValueError):
                d.validate(ident,record,root)
            record["operatorAcknowledgesManualReview"] = True
            with self.assertRaises(ValueError):
                d.verify_release_template(ident, record)
            record["device"] = {"model":"iPhone 16 Pro","os":"27.2.1","installation":"SideStore","testedDate":"2026-11-28"}
            record["versions"] = {key:"test-only" for key in d.VERSIONS}
            pending = d.validate(ident,record,root)
            self.assertFalse(pending["operatorReviewComplete"])
            self.assertFalse(pending["productionReady"])
            for key in d.required("ubuntu"):
                path = root / (key + ".txt")
                path.write_text("Synthetic unit test only. Not a real device observation.")
                record["checks"][key] = {"passed":True,"evidenceSha256":d.sha_file(path)}
            verified = d.validate(ident,record,root)
            self.assertTrue(verified["operatorReviewComplete"])
            self.assertFalse(verified["physicalDeviceIndependentlyVerified"])
            self.assertFalse(verified["productionReady"])
            (root / "agy_google_auth_real.txt").write_text("changed")
            with self.assertRaises(ValueError):
                d.validate(ident,record,root)
            ipa.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                d.package_identity(pkg)

if __name__ == "__main__":
    unittest.main()
