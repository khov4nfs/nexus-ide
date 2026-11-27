"""Apply only reviewed edits to the pinned upstream; safe for repeat builds."""
import json
from pathlib import Path
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
manifest = json.loads((Path(__file__).resolve().parent.parent / "RuntimeBridge/upstream-patches.json").read_text())
def git(*args):
    return subprocess.check_output(["git", "-C", str(root), *args])
if git("rev-parse", "HEAD").decode().strip() != manifest["commit"]:
    raise SystemExit("Unexpected upstream commit")
if git("diff", "--cached", "--name-only", "-z"):
    raise SystemExit("Refusing staged upstream edits")
changed = set(git("diff", "--name-only", "-z", "HEAD").decode().strip("\0").split("\0")) - {""}
if changed - set(manifest["changes"]):
    raise SystemExit(f"Unexpected upstream changes: {changed - set(manifest['changes'])}")
prepared = {}
for path, edits in manifest["changes"].items():
    original = git("show", f"HEAD:{path}").decode()
    expected = original
    for old, new in edits:
        if expected.count(old) != 1:
            raise SystemExit(f"Patch context mismatch: {path}")
        expected = expected.replace(old, new, 1)
    target = root / path
    actual = target.read_bytes()
    if actual not in (original.encode(), expected.encode()):
        raise SystemExit(f"Refusing modified upstream file: {path}")
    prepared[target] = expected.encode()
for target, content in prepared.items():
    if target.read_bytes() != content:
        target.write_bytes(content)
print("Verified pinned runtime patches:", len(prepared), "files")
