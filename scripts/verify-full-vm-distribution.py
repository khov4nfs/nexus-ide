"""Fail-closed owner review gate for a full-VM test candidate.

Checks recorded decisions and byte identity, not legal sufficiency or authorship.
No record is generated or automatically approved by CI.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
from urllib.parse import urlsplit

def sha(path):
    if not path.is_file() or path.is_symlink():
        raise ValueError("Expected regular reviewed file")
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def review(root):
    path = root / "docs/full-vm-distribution-review.json"
    record = json.loads(path.read_text())
    if record.get("ownerApproved") is not True:
        raise ValueError("Owner distribution approval required")
    if record.get("combinedWorkLicenseReviewed") is not True or record.get("allDependenciesReviewed") is not True or record.get("correspondingSourceReviewed") is not True:
        raise ValueError("Combined license, dependencies and source review required")
    for key in ("sourceInventorySHA256", "sourceArchiveSHA256", "repositoryLicenseSHA256"):
        if not re.fullmatch("[a-f0-9]{64}", record.get(key, "")):
            raise ValueError("Invalid " + key)
    if sha(root / "LICENSE") != record["repositoryLicenseSHA256"]:
        raise ValueError("Repository license changed or missing")
    url = urlsplit(record.get("sourceArchiveURL", ""))
    if url.scheme != "https" or not url.hostname or url.username or url.password or url.fragment or url.query:
        raise ValueError("Use a public, stable HTTPS corresponding-source archive URL")
    return record

if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    p.add_argument("--inventory", type=Path)
    p.add_argument("--archive", type=Path)
    p.add_argument("--print-source-url", action="store_true")
    a = p.parse_args()
    record = review(a.root)
    if a.inventory and sha(a.inventory) != record["sourceInventorySHA256"]:
        raise ValueError("Actual dependency inventory differs from owner-reviewed inventory")
    if a.archive and sha(a.archive) != record["sourceArchiveSHA256"]:
        raise ValueError("Corresponding-source archive hash mismatch")
    if a.print_source_url:
        print(record["sourceArchiveURL"])
    else:
        print("Owner review fields and supplied hashes verified; legal sufficiency not established by CI.")
