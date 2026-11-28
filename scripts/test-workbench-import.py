import hashlib
import importlib.util
import json
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("workbench", Path(__file__).with_name("prepare-workbench-rootfs.py"))
workbench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workbench)

def add(t, name, data=None, link=None):
    m = tarfile.TarInfo(name)
    m.mode = 0o755
    if link is not None:
        m.type = tarfile.SYMTYPE
        m.linkname = link
        t.addfile(m)
    elif data is None:
        m.type = tarfile.DIRTYPE
        t.addfile(m)
    else:
        m.size = len(data)
        t.addfile(m, io.BytesIO(data))

class OverlayTests(unittest.TestCase):
    def test_overlay_and_reject_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = root / "base.tar.gz"
            with tarfile.open(base, "w:gz") as t:
                add(t, "./opt")
            for escape in (False, True):
                package = root / f"package-{escape}.tar.gz"
                prefix = workbench.PREFIX
                with tarfile.open(package, "w:gz") as t:
                    add(t, prefix)
                    add(t, prefix + "/bin")
                    add(t, prefix + "/bin/openvscode-server", b"fixture")
                    add(t, prefix + "/node", b"fixture")
                    add(t, prefix + "/LICENSE", b"fixture")
                    add(t, prefix + "/bin/link", link="../../outside" if escape else "../node")
                out = root / f"out-{escape}.tar.gz"
                if escape:
                    with self.assertRaises(ValueError):
                        workbench.overlay(base, package, out)
                else:
                    workbench.overlay(base, package, out)
                    with tarfile.open(out, "r:gz") as t:
                        p = "./" + workbench.DEST
                        self.assertEqual(t.extractfile(p + "/node").read(), b"fixture")
                        self.assertEqual(t.extractfile("./" + workbench.VSIX_SLOT).read(), workbench.VSIX_MARKER)
                        self.assertEqual(t.getmember(p + "/bin/link").linkname, "../node")

    def test_missing_license_requires_pinned_source_documents(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = root / "base.tar.gz"
            package = root / "package.tar.gz"
            with tarfile.open(base, "w:gz") as t:
                add(t, "./opt")
            with tarfile.open(package, "w:gz") as t:
                prefix = workbench.PREFIX
                add(t, prefix)
                add(t, prefix + "/bin")
                add(t, prefix + "/bin/openvscode-server", b"fixture")
                add(t, prefix + "/node", b"fixture")
            with self.assertRaisesRegex(ValueError, "Missing release license"):
                workbench.overlay(base, package, root / "reject.tar.gz")
            docs = {"LICENSE.txt": b"fixture license", "ThirdPartyNotices.txt": b"fixture notices"}
            out = root / "accept.tar.gz"
            workbench.overlay(base, package, out, docs)
            with tarfile.open(out, "r:gz") as t:
                prefix = "./" + workbench.DEST + "/"
                self.assertEqual(t.extractfile(prefix + "LICENSE.txt").read(), docs["LICENSE.txt"])
                for name, data in docs.items():
                    self.assertEqual(t.extractfile(prefix + "NEXUS-UPSTREAM-" + name).read(), data)
            fake = root / "fake-license"
            fake.write_bytes(b"x" * workbench.SOURCE_DOCUMENTS["LICENSE.txt"][0])
            with self.assertRaisesRegex(ValueError, "Git blob mismatch"):
                workbench.verify_source_document(fake, "LICENSE.txt")


    def test_reviewed_optional_lsp_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = root / "base.tar.gz"
            with tarfile.open(base, "w:gz") as t:
                add(t, "./opt")
                add(t, "./opt/nexus")
            entry = workbench.LSP_ENTRIES["typescript"]
            data = b"synthetic JS fixture, not a language server"
            for bad in ("none", "digest", "symlink", "escape"):
                bundle = root / f"bundle-{bad}.tar.gz"
                digest = hashlib.sha256(data).hexdigest() if bad != "digest" else "0" * 64
                record = {"schemaVersion":1,"architecture":"arm64","servers":{"typescript":{"version":"4.3.4","entrySHA256":digest}}}
                with tarfile.open(bundle, "w:gz") as t:
                    for parent in reversed(Path(entry).parents):
                        if str(parent) != ".":
                            add(t, parent.as_posix())
                    add(t, entry, data)
                    add(t, "notices")
                    add(t, "notices/README.txt", b"synthetic test only")
                    add(t, "manifest.json", json.dumps(record).encode())
                    if bad == "symlink":
                        add(t, "bin")
                        add(t, "bin/unsafe", link="/etc/passwd")
                    if bad == "escape":
                        add(t, "../escape", b"x")
                out = root / f"output-{bad}.tar.gz"
                if bad != "none":
                    with self.assertRaises(ValueError):
                        workbench.overlay_reviewed_lsp(base,bundle,out)
                else:
                    inventory=workbench.overlay_reviewed_lsp(base,bundle,out)
                    self.assertEqual(inventory,record["servers"])
                    with tarfile.open(out,"r:gz") as t:
                        self.assertEqual(t.extractfile("./"+workbench.LSP_DEST+"/"+entry).read(),data)

if __name__ == "__main__":
    unittest.main()
