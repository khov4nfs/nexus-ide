#!/usr/bin/env python3
"""Package and verify a real unsigned iPhoneOS arm64 app, never a simulator build."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import stat
import struct
import subprocess
import tempfile
import zipfile

VARIANTS = {
    "native": "dev.nexuside.local",
    "ubuntu": "dev.nexuside.ubuntu.lab",
    "workbench": "dev.nexuside.ubuntu.lab",
}
WORKBENCH_SHA = "36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc"

def validate_macho(data):
    # Xcode device builds are intentionally single-slice ARM64. Reject FAT,
    # simulator, malformed and unknown deployment load commands.
    if len(data) < 32:
        raise ValueError("Truncated Mach-O")
    magic, cpu, subtype, kind, count, size, flags, reserved = struct.unpack_from("<8I", data)
    if magic != 0xfeedfacf or cpu != 0x0100000c or kind != 2 or count > 4096 or size > len(data)-32:
        raise ValueError("Expected thin ARM64 Mach-O executable")
    pos, end, platforms = 32, 32+size, []
    for _ in range(count):
        if pos+8 > end:
            raise ValueError("Truncated load command")
        cmd, length = struct.unpack_from("<II", data, pos)
        if length < 8 or length % 8 or pos+length > end:
            raise ValueError("Malformed load command")
        if cmd == 0x32:
            if length < 24:
                raise ValueError("Malformed LC_BUILD_VERSION")
            platforms.append(struct.unpack_from("<I", data, pos+8)[0])
        pos += length
    if pos != end or platforms != [2]:
        raise ValueError("Expected LC_BUILD_VERSION platform iOS (not simulator)")

def verify(path, variant):
    with zipfile.ZipFile(path) as z:
        infos = z.infolist()
        names = {i.filename for i in infos}
        if len(names) != len(infos):
            raise ValueError("Duplicate ZIP entry")
        apps = set()
        for i in infos:
            p = PurePosixPath(i.filename)
            if p.is_absolute() or ".." in p.parts or "\\" in i.filename or not p.parts or p.parts[0] != "Payload":
                raise ValueError("Unexpected IPA path")
            if len(p.parts) >= 2:
                if not p.parts[1].endswith(".app"):
                    raise ValueError("Unexpected Payload entry")
                apps.add(p.parts[1])
            if stat.S_ISLNK(i.external_attr >> 16):
                raise ValueError("Unexpected physical symlink in app; review packaging")
        if len(apps) != 1:
            raise ValueError("Expected one application")
        prefix = "Payload/" + apps.pop() + "/"
        info = plistlib.loads(z.read(prefix+"Info.plist"))
        if info.get("CFBundleIdentifier") != VARIANTS[variant] or info.get("CFBundleSupportedPlatforms") != ["iPhoneOS"]:
            raise ValueError("Wrong bundle or platform")
        if not isinstance(info.get("UILaunchScreen"), dict):
            raise ValueError("Missing generated UILaunchScreen; reject legacy letterboxed IPA")
        if info.get("MinimumOSVersion") != "17.0":
            raise ValueError("Unexpected minimum OS")
        exe = info.get("CFBundleExecutable", "")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+",exe) or exe in (".",".."):
            raise ValueError("Invalid executable name")
        binary = z.getinfo(prefix+exe)
        if not binary.external_attr >> 16 & stat.S_IXUSR:
            raise ValueError("Executable permission missing")
        with z.open(binary) as stream:
            header = stream.read(32)
            if len(header) != 32:
                raise ValueError("Truncated executable")
            size = struct.unpack_from("<I",header,20)[0]
            if size > 1024*1024:
                raise ValueError("Excessive load commands")
            validate_macho(header+stream.read(size))
        if prefix+"embedded.mobileprovision" in names:
            raise ValueError("Unexpected provisioning profile in public unsigned build")
        slot = prefix+"ubuntu-fs/data/opt/nexus/agy"
        if prefix+"ubuntu-fs/agy.sha256" in names or any(n.startswith(slot) and n != slot for n in names):
            raise ValueError("Proprietary AGY must not be bundled in public candidate")
        if slot in names and (z.getinfo(slot).file_size != len(b"NEXUS_AGY_IMPORT_SLOT_V1\n") or
                             z.read(slot) != b"NEXUS_AGY_IMPORT_SLOT_V1\n"):
            raise ValueError("Expected empty AGY import slot, not a binary")
        guest = prefix+"ubuntu-fs/"
        has_guest = guest+"meta.db" in names
        if has_guest != (variant != "native"):
            raise ValueError("Incorrect embedded guest assets")
        if has_guest:
            if not z.read(prefix+'SwiftTerm-LICENSE.txt').strip(): raise ValueError('Missing SwiftTerm license')
            if b'version=v1.19.0\ncommit=464df52' not in z.read(prefix+'SwiftTerm-provenance.txt'): raise ValueError('Missing SwiftTerm provenance')
            if slot not in names:
                raise ValueError("Missing AGY import slot")
            with z.open(guest+"meta.db") as db:
                if db.read(16) != b"SQLite format 3\0":
                    raise ValueError("Invalid fakefs database")
            if guest+"meta.db-wal" in names or guest+"meta.db-shm" in names:
                raise ValueError("Uncheckpointed fakefs")
            if not any(n.startswith(guest+"data/") for n in names):
                raise ValueError("Missing fakefs data")
        if variant == "workbench":
            if z.read(guest+"workbench.sha256").decode().strip() != WORKBENCH_SHA:
                raise ValueError("Missing pinned workbench")
            for entry in ("data/opt/nexus/workbench/node","data/opt/nexus/workbench/bin/openvscode-server"):
                if guest+entry not in names:
                    raise ValueError("Missing workbench executable")
        elif guest+"workbench.sha256" in names:
            raise ValueError("Unexpected workbench in plain build")
        if z.testzip() is not None:
            raise ValueError("ZIP CRC failed")
        return {
            "bundleIdentifier":info["CFBundleIdentifier"],
            "version":info["CFBundleShortVersionString"],
            "build":info["CFBundleVersion"],
            "minimumOS":info["MinimumOSVersion"],
            "architecture":"arm64","platform":"iPhoneOS",
            "signing":"unsigned; must be signed by a legitimate installer",
            "variant":variant,"physicalDeviceVerified":False,
            "agyBundled":False,"agyImportSlot":has_guest,"guestBundled":has_guest,
        }

def package(app, output, variant, commit):
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Expected exact source commit")
    if not app.is_dir() or app.is_symlink() or app.suffix != ".app":
        raise ValueError("Expected built app directory")
    output.mkdir(parents=True, exist_ok=True)
    name = f"NexusIDE-{variant}-{commit[:12]}-unsigned.ipa"
    dest = output/name
    for candidate in (dest, output/"SHA256SUMS", output/"build.json"):
        if candidate.exists() or candidate.is_symlink():
            raise ValueError("Refusing to replace existing artifact")
    # zip -y preserves permissions and does not follow links. The verifier
    # rejects links rather than silently resolving them outside the bundle.
    with tempfile.TemporaryDirectory(prefix="nexus-ipa-") as d:
        root = Path(d)
        payload = root/"Payload"
        payload.mkdir()
        subprocess.run(["/usr/bin/ditto","--norsrc",str(app),str(payload/app.name)],check=True)
        archive = root/name
        subprocess.run(["/usr/bin/zip","-q","-r","-y",str(archive),"Payload"],cwd=root,check=True)
        report = verify(archive,variant)
        with archive.open("rb") as f:
            digest = hashlib.file_digest(f,"sha256").hexdigest()
        report.update({"commit":commit,"sha256":digest,"file":name,"bytes":archive.stat().st_size})
        import shutil
        shutil.copyfile(archive,dest)
    (output/"SHA256SUMS").write_text(f"{digest}  {name}\n")
    (output/"build.json").write_text(json.dumps(report,indent=2,sort_keys=True)+"\n")
    print(json.dumps(report,indent=2,sort_keys=True))

if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--app",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    p.add_argument("--variant",choices=VARIANTS,required=True)
    p.add_argument("--commit",required=True)
    a=p.parse_args()
    package(a.app,a.output,a.variant,a.commit)
