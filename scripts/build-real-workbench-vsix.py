"""Build the reviewable real-pylsp acceptance VSIX; no synthetic LSP server."""
import subprocess
import sys
import tempfile
from pathlib import Path
import zipfile

here = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory() as tmp:
    fixture = Path(tmp) / "fixture.vsix"
    subprocess.run([sys.executable, str(here / "build-test-vsix.py"), str(fixture)], check=True)
    with zipfile.ZipFile(fixture) as source, zipfile.ZipFile(Path(sys.argv[1]), "x", zipfile.ZIP_DEFLATED) as target:
        for entry in source.infolist():
            if entry.filename in ("extension/extension.js", "extension/server.js"):
                continue
            target.writestr(entry.filename, source.read(entry))
        target.writestr("extension/extension.js", (here / "workbench-real-lsp-extension.js").read_bytes())
