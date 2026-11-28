"""Stage only the reviewed QEMU iOS framework dependency closure and firmware."""
import hashlib
import json
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys

def command(*args):
    return subprocess.check_output(args, text=True)

def stage(sysroot, target, utm, qemu):
    if target.exists():
        raise ValueError("Refusing to replace staged VM")
    frameworks = sysroot / "Frameworks"
    entry = "qemu-aarch64-softmmu"
    pending, seen = [entry], set()
    binaries = {}
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        if not re.fullmatch(r"[A-Za-z0-9_.+-]+", name):
            raise ValueError("Invalid framework name")
        seen.add(name)
        folder = frameworks / (name + ".framework")
        binary = folder / name
        if not binary.is_file() or binary.is_symlink():
            raise ValueError("Missing regular framework binary " + name)
        info = plistlib.loads((folder / "Info.plist").read_bytes())
        if info["CFBundleExecutable"] != name:
            raise ValueError("Framework identity mismatch")
        if command("lipo", "-archs", str(binary)).strip() != "arm64":
            raise ValueError("Not single slice arm64: " + name)
        platform = command("vtool", "-show-build-version", str(binary))
        if not re.search(r"^\s*platform IOS\s*$", platform, re.M):
            raise ValueError("Not iPhoneOS: " + name)
        deps = command("otool", "-L", str(binary)).splitlines()[1:]
        for dep in deps:
            link = dep.strip().split(" (", 1)[0]
            if "Hypervisor.framework" in link or "/PrivateFrameworks/" in link:
                raise ValueError("Forbidden framework dependency: " + link)
            if link.startswith(("/usr/lib/", "/System/Library/Frameworks/")):
                continue
            match = re.fullmatch(r"@rpath/([^/]+)\.framework/\1", link)
            if not match:
                raise ValueError("Unexpected dependency: " + link)
            pending.append(match.group(1))
        binaries[name] = binary
    symbols = command("nm", "-gU", str(binaries[entry]))
    if not re.search(r"\b_nexus_qemu_tci_run$", symbols, re.M):
        raise ValueError("Missing interpreter-only integration entrypoint")
    dest = target / "Frameworks"
    dest.mkdir(parents=True)
    hashes = {}
    for name, binary in sorted(binaries.items()):
        folder = dest / (name + ".framework")
        folder.mkdir()
        shutil.copy2(binary, folder / name)
        shutil.copy2(binary.parent / "Info.plist", folder / "Info.plist")
        # The framework is unsigned at this point; legitimate app signing is later.
        with (folder / name).open("rb") as stream:
            hashes[name] = hashlib.file_digest(stream, "sha256").hexdigest()
    firmware = sysroot / "share/qemu"
    if not firmware.is_dir():
        raise ValueError("Missing QEMU firmware/data directory")
    shutil.copytree(firmware, target / "Firmware", symlinks=False)
    for p in (target / "Firmware").rglob("*"):
        if p.is_symlink() or not (p.is_file() or p.is_dir()):
            raise ValueError("Unexpected firmware entry")
    root = Path(__file__).resolve().parent.parent
    for name, path in (("QEMU-COPYING", root / "Vendor/nexus-qemu/COPYING"),
                       ("QEMU-LICENSE", root / "Vendor/nexus-qemu/LICENSE"),
                       ("UTM-LICENSE", root / "Vendor/nexus-utm/LICENSE")):
        shutil.copy2(path, target / name)
    # Inventory the actual fetched archives and Git dependency revisions.
    # This is an audit input, not a legal approval or source offer.
    utm_root = root / "Vendor/nexus-utm"
    qemu_root = root / "Vendor/nexus-qemu"
    def sha(path):
        with path.open("rb") as stream:
            return hashlib.file_digest(stream, "sha256").hexdigest()
    sources = {}
    for build in sorted(utm_root.glob("build-*")):
        for item in sorted(build.iterdir()):
            if item.is_file() and item.name.endswith((".tar.gz", ".tar.xz", ".tar.bz2", ".tgz")):
                sources[item.name] = sha(item)
            elif item.is_dir() and (item / ".git").exists():
                sources[item.name] = command("git", "-C", str(item), "rev-parse", "HEAD").strip()
    for item in sorted((qemu_root / "subprojects").glob("*")):
        if item.is_dir() and (item / ".git").exists():
            sources["qemu-subproject/" + item.name] = command("git", "-C", str(item), "rev-parse", "HEAD").strip()
    for item in sorted((qemu_root / "subprojects/packagecache").glob("*")):
        if item.is_file():
            sources["qemu-packagecache/" + item.name] = sha(item)
    notices = target / "DependencyNotices"
    notices.mkdir()
    notice_hashes = {}
    roots = [("qemu", qemu_root), ("utm", utm_root)]
    for build in sorted(utm_root.glob("build-*")):
        roots.extend((item.name, item) for item in sorted(build.iterdir()) if item.is_dir())
    for name, directory in roots:
        for item in sorted(directory.iterdir()):
            if item.is_file() and not item.is_symlink() and re.match(r"^(COPYING|LICENSE|LICENCE|COPYRIGHT|NOTICE)([._-].*)?$", item.name, re.I):
                if item.stat().st_size > 2_000_000:
                    raise ValueError("Unexpected notice size")
                dest_notice = notices / (name + "-" + item.name)
                shutil.copy2(item, dest_notice)
                notice_hashes[dest_notice.name] = sha(dest_notice)
    for item in sorted((qemu_root / "pc-bios").glob("*")):
        if item.is_file() and (item.name == "README" or "license" in item.name.lower()):
            dest_notice = notices / ("firmware-" + item.name)
            shutil.copy2(item, dest_notice)
            notice_hashes[dest_notice.name] = sha(dest_notice)
    firmware_hashes = {p.relative_to(target / "Firmware").as_posix(): sha(p)
                       for p in sorted((target / "Firmware").rglob("*")) if p.is_file()}
    inventory = {"utmCommit": utm, "qemuCommit": qemu, "sources": sources,
                 "noticeSHA256": notice_hashes, "firmwareSHA256": firmware_hashes,
                 "qemuPatchSHA256": sha(utm_root / "patches/qemu-10.0.12-utm.patch"),
                 "buildScriptSHA256": sha(root / "scripts/build-ios-vm.sh")}
    (target / "source-inventory.json").write_text(json.dumps(inventory, sort_keys=True, indent=2) + "\n")
    (target / "provenance.json").write_text(json.dumps({
        "utmCommit": utm, "qemuCommit": qemu, "architecture": "arm64",
        "platform": "iPhoneOS", "interpreterOnlyEntryPoint": True,
        "frameworkSHA256": hashes, "sourceInventorySHA256": sha(target / "source-inventory.json"), "physicalDeviceVerified": False,
        "distributionReviewComplete": False
    }, indent=2) + "\n")
    print("Staged device framework closure:", ", ".join(sorted(hashes)))

if __name__ == "__main__":
    stage(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4])
