"""Prepare a pinned local OpenVSCode ARM64 fakefs, optionally with reviewed AGY.

No archive extraction into the host, install script, package execution or
automatic download. A published GitHub asset digest is an integrity pin,
not an independent signature. Do not distribute proprietary AGY.
"""
import argparse
import copy
import importlib.util
from pathlib import Path, PurePosixPath
import posixpath
import shutil
import subprocess
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("agy_import", HERE / "prepare-agy-rootfs.py")
agy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agy)
VERSION = "1.109.5"
PREFIX = f"openvscode-server-v{VERSION}-linux-arm64"
SHA = "36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc"
ASSET = f"https://github.com/gitpod-io/openvscode-server/releases/download/openvscode-server-v{VERSION}/{PREFIX}.tar.gz"
DEST = "opt/nexus/workbench"

def add_dir(dst, name):
    info = tarfile.TarInfo("./" + name)
    info.type = tarfile.DIRTYPE
    info.mode = 0o755
    dst.addfile(info)

def overlay(original, package, output):
    with tarfile.open(original, "r:gz") as src, tarfile.open(package, "r:gz") as pkg, tarfile.open(output, "x:gz") as dst:
        dirs = set()
        for member in src:
            p = PurePosixPath(member.name)
            if p.is_absolute() or ".." in p.parts:
                raise ValueError("Unsafe base path")
            name = p.as_posix()
            if name == DEST or name.startswith(DEST + "/"):
                raise ValueError("Workbench destination already exists")
            if name in ("opt", "opt/nexus"):
                if not member.isdir() or name in dirs:
                    raise ValueError("Invalid base directory")
                dirs.add(name)
            if member.isfile():
                with src.extractfile(member) as stream:
                    dst.addfile(member, stream)
            else:
                dst.addfile(member)
        for directory in ("opt", "opt/nexus"):
            if directory not in dirs:
                add_dir(dst, directory)
        add_dir(dst, DEST)
        members = {}
        total = 0
        for member in pkg:
            p = PurePosixPath(member.name)
            if p.is_absolute() or ".." in p.parts or not p.parts or p.parts[0] != PREFIX:
                raise ValueError("Invalid release path")
            relative = PurePosixPath(*p.parts[1:]).as_posix()
            if relative == ".":
                if not member.isdir():
                    raise ValueError("Invalid release root")
                continue
            if relative in members or len(members) >= 100_000:
                raise ValueError("Duplicate or excessive release entries")
            if not (member.isdir() or member.isfile() or member.issym()):
                raise ValueError("Unsupported archive entry (including hardlinks)")
            if member.issym():
                link = PurePosixPath(member.linkname)
                target = posixpath.normpath(posixpath.join(posixpath.dirname(relative), member.linkname))
                if not member.linkname or link.is_absolute() or target == ".." or target.startswith("../"):
                    raise ValueError("Symlink escapes release")
            total += member.size if member.isfile() else 0
            if total > 1024**3:
                raise ValueError("Expanded release too large")
            members[relative] = member
        for relative in members:
            for parent in PurePosixPath(relative).parents:
                name = parent.as_posix()
                if name != "." and (name not in members or not members[name].isdir()):
                    raise ValueError("Missing or non-directory ancestor")
        for required in ("bin/openvscode-server", "node", "LICENSE"):
            # The release may name its license LICENSE.txt instead.
            if required == "LICENSE":
                if not any(name in members and members[name].isfile() for name in ("LICENSE", "LICENSE.txt")):
                    raise ValueError("Missing release license")
            elif required not in members or not members[required].isfile() or not members[required].mode & 0o111:
                raise ValueError("Missing executable: " + required)
        for relative, member in members.items():
            info = copy.copy(member)
            info.name = "./" + DEST + "/" + relative
            info.pax_headers = {}
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            info.mode &= 0o777
            if member.isfile():
                with pkg.extractfile(member) as stream:
                    dst.addfile(info, stream)
            else:
                dst.addfile(info)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--confirm-local-use", action="store_true", required=True)
    parser.add_argument("--agy-binary", type=Path)
    parser.add_argument("--agy-sha256")
    args = parser.parse_args()
    if bool(args.agy_binary) != bool(args.agy_sha256):
        parser.error("Provide both optional AGY arguments or neither")
    if not args.archive.is_file() or args.archive.is_symlink() or args.archive.stat().st_size != 74488835:
        raise SystemExit("Expected pinned regular release archive")
    target = agy.BUILD / "workbench-fs"
    if target.exists() or target.is_symlink():
        raise SystemExit("Refusing to overwrite workbench-fs")
    if subprocess.check_output(["git", "-C", str(agy.ROOT / "Vendor/litter-ish"), "rev-parse", "HEAD"], text=True).strip() != agy.PIN:
        raise SystemExit("Unexpected runtime commit")
    subprocess.run(["python3", str(HERE / "apply-runtime-patches.py"), str(agy.ROOT / "Vendor/litter-ish")], check=True)
    if agy.digest(agy.UBUNTU) != agy.UBUNTU_SHA:
        raise SystemExit("Ubuntu archive digest mismatch")
    converter = agy.BUILD / "host-fakefsify/tools/fakefsify"
    if not converter.is_file():
        raise SystemExit("Build pinned runtime first")
    with tempfile.TemporaryDirectory(prefix="workbench-import-", dir=agy.BUILD) as tmp:
        stage = Path(tmp)
        package = stage / "release.tar.gz"
        shutil.copyfile(args.archive, package)
        if agy.digest(package) != SHA:
            raise SystemExit("OpenVSCode release SHA-256 mismatch")
        original = agy.UBUNTU
        if args.agy_binary:
            agy.validate_binary(args.agy_binary, args.agy_sha256)
            binary = stage / "agy"
            shutil.copyfile(args.agy_binary, binary)
            agy.validate_binary(binary, args.agy_sha256)
            original = stage / "with-agy.tar.gz"
            agy.overlay_archive(agy.UBUNTU, binary, original)
        else:
            original = stage / "with-slot.tar.gz"
            agy.overlay_import_slot(agy.UBUNTU, original)
        archive = stage / "rootfs.tar.gz"
        overlay(original, package, archive)
        fs = stage / "fs"
        subprocess.run([str(converter), str(archive), str(fs)], check=True)
        subprocess.run(["sqlite3", str(fs / "meta.db"), "PRAGMA wal_checkpoint(TRUNCATE);"], check=True)
        for suffix in ("-wal", "-shm"):
            (fs / ("meta.db" + suffix)).unlink(missing_ok=True)
        node = fs / "data" / DEST / "node"
        agy.validate_binary(node, agy.digest(node))
        launcher = fs / "data" / DEST / "bin/openvscode-server"
        (fs / "workbench.sha256").write_text(SHA + "\n")
        (fs / "workbench-launcher.sha256").write_text(agy.digest(launcher) + "\n")
        (fs / "workbench-node.sha256").write_text(agy.digest(node) + "\n")
        if args.agy_binary:
            if agy.digest(fs / "data/opt/nexus/agy") != args.agy_sha256:
                raise SystemExit("AGY digest mismatch after conversion")
            (fs / "agy.sha256").write_text(args.agy_sha256 + "\n")
        (fs / "provenance.txt").write_text(f"ubuntu_sha256={agy.UBUNTU_SHA}\nworkbench_source={ASSET}\nworkbench_sha256={SHA}\n" + (f"agy_sha256={args.agy_sha256}\n" if args.agy_binary else ""))
        if target.exists() or target.is_symlink():
            raise SystemExit("Target appeared during import")
        fs.rename(target)
    print("Prepared local workbench fakefs; execution not verified:", target)

if __name__ == "__main__":
    main()
