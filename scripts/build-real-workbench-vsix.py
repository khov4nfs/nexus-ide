"""Build the reviewable real-pylsp acceptance VSIX; no synthetic LSP server."""
import json
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
            if entry.filename == "extension/package.json":
                manifest = json.loads(source.read(entry))
                manifest.update(displayName="Nexus persistent language servers", description="Guest-local reviewed persistent language servers and project edits", activationEvents=["onLanguage:python", "onLanguage:c", "onLanguage:cpp", "onLanguage:typescript", "onLanguage:typescriptreact", "onLanguage:javascript", "onLanguage:javascriptreact", "onLanguage:rust", "onLanguage:go", "onLanguage:shellscript", "onCommand:nexus.lsp.approve", "onCommand:nexus.lsp.rename", "onCommand:nexus.lsp.refactor", "onCommand:nexus.lsp.index", "onCommand:nexus.lsp.projectFixture", "onCommand:nexus.fixture.open", "onCommand:nexus.fixture.cpp", "onCommand:nexus.fixture.c", "onCommand:nexus.lsp.stop", "onCommand:nexus.lsp.restart"], capabilities={"untrustedWorkspaces":{"supported":False}})
                manifest["contributes"]["commands"] += [{"command":"nexus.fixture.cpp","title":"Nexus: Open C++ CI Fixture"},{"command":"nexus.fixture.c","title":"Nexus: Open C CI Fixture"},{"command":"nexus.lsp.stop","title":"Nexus: Stop Language Servers"},{"command":"nexus.lsp.restart","title":"Nexus: Restart Language Servers"},{"command":"nexus.lsp.approve","title":"Nexus: Enable Reviewed Language Server"},{"command":"nexus.lsp.rename","title":"Nexus: Review Symbol Rename"},{"command":"nexus.lsp.refactor","title":"Nexus: Review Refactoring"},{"command":"nexus.lsp.index","title":"Nexus: Enable Project Indexing"},{"command":"nexus.lsp.projectFixture","title":"Nexus: Run Synthetic Cross-file Acceptance"}]
                target.writestr(entry.filename, json.dumps(manifest))
            else:
                target.writestr(entry.filename, source.read(entry))
        target.writestr("extension/extension.js", (here / "workbench-real-lsp-extension.js").read_bytes())
        target.writestr("extension/lsp-session.js", (here / "workbench-lsp-session.js").read_bytes())
        target.writestr("extension/lsp-edits.js", (here / "workbench-lsp-edits.js").read_bytes())
