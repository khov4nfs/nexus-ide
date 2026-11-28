import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("vm_review", Path(__file__).with_name("verify-full-vm-distribution.py"))
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)

class Tests(unittest.TestCase):
    def test_missing_unapproved_or_changed_license(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs").mkdir()
            record = root / "docs/full-vm-distribution-review.json"
            license = root / "LICENSE"
            with self.assertRaises(FileNotFoundError):
                review.review(root)
            license.write_text("synthetic fixture, not a real license approval")
            approved = {
                "ownerApproved": False, "combinedWorkLicenseReviewed": True,
                "allDependenciesReviewed": True, "correspondingSourceReviewed": True,
                "sourceInventorySHA256": "a" * 64, "sourceArchiveSHA256": "b" * 64,
                "repositoryLicenseSHA256": hashlib.sha256(license.read_bytes()).hexdigest(),
                "sourceArchiveURL": "https://example.invalid/source.tar.gz"
            }
            record.write_text(json.dumps(approved))
            with self.assertRaises(ValueError):
                review.review(root)
            approved["ownerApproved"] = True
            record.write_text(json.dumps(approved))
            self.assertEqual(review.review(root), approved)
            license.write_text("changed")
            with self.assertRaises(ValueError):
                review.review(root)

if __name__ == "__main__":
    unittest.main()
