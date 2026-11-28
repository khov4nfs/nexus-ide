"""Prepare exact npm-locked TypeScript/JS tools; never execute install scripts."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

here = Path(__file__).resolve().parent
output = Path(sys.argv[1]).resolve()
if output.exists():
    raise SystemExit("Refusing to overwrite bundle")
with tempfile.TemporaryDirectory(prefix="nexus-ts-bundle-") as tmp:
    root = Path(tmp)
    tools = root / "tools"
    package = tools / "typescript"
    package.mkdir(parents=True)
    for name in ("package.json", "package-lock.json"):
        shutil.copyfile(here / "lsp-typescript" / name, package / name)
    subprocess.run(["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund",
                    "--cache", str(root / "cache")], cwd=package, check=True)
    links = package / "node_modules/.bin"
    if links.exists():
        shutil.rmtree(links)
    entry = package / "node_modules/typescript-language-server/lib/cli.mjs"
    digest = hashlib.sha256(entry.read_bytes()).hexdigest()
    if digest != "557a242e685e6b49967597a8d33934190f391090c763a0d344b9e7211e55b5dd":
        raise ValueError("Pinned TypeScript server entry mismatch")
    notices = tools / "notices"
    notices.mkdir()
    for name in ("typescript", "typescript-language-server"):
        source = package / "node_modules" / name
        count = 0
        for file in source.iterdir():
            if file.is_file() and file.name.lower().startswith(("license", "notice", "thirdpartynotices")):
                shutil.copyfile(file, notices / (name + "-" + file.name))
                count += 1
        if not count:
            raise ValueError("Missing package license: " + name)
    (tools / "manifest.json").write_text(json.dumps({"schemaVersion":1,"architecture":"arm64",
        "servers":{"typescript":{"version":"4.3.4","entrySHA256":digest}}},sort_keys=True)+"\n")
    # Real host regression runs against the same byte-identical packages
    # before the guest bundle is staged. These packages are JS, not host ELF.
    subprocess.run(["node", str(here / "test-real-typescript-lsp.js"), str(tools)],check=True)
    with tarfile.open(output, "x:gz") as archive:
        for file in sorted(tools.rglob("*")):
            if file.is_symlink() or not (file.is_file() or file.is_dir()):
                raise ValueError("Unexpected package archive entry")
            info = archive.gettarinfo(str(file), arcname=file.relative_to(tools).as_posix())
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            info.mtime = 0
            info.mode &= 0o777
            if file.is_file():
                with file.open("rb") as stream:
                    archive.addfile(info, stream)
            else:
                archive.addfile(info)
print(output)
