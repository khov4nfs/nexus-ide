"""Generate a tiny local CI-only extension. No marketplace, dependencies or network."""
import json
from pathlib import Path
import sys
import zipfile

manifest = {"name":"fixture","publisher":"nexus","version":"0.0.1","displayName":"Nexus CI fixture","engines":{"vscode":"^1.80.0"},"main":"./extension.js","activationEvents":["onCommand:nexus.fixture.open"],"contributes":{"commands":[{"command":"nexus.fixture.open","title":"Nexus: Open CI Fixture"}]}}
vsix = """<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011">
  <Metadata><Identity Language="en-US" Id="fixture" Version="0.0.1" Publisher="nexus"/>
    <DisplayName>Nexus CI fixture</DisplayName><Description xml:space="preserve">Local CI-only fixture</Description></Metadata>
  <Installation><InstallationTarget Id="Microsoft.VisualStudio.Code"/></Installation>
  <Dependencies/>
  <Assets><Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true"/></Assets>
</PackageManifest>"""
types = """<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="json" ContentType="application/json"/><Default Extension="js" ContentType="application/javascript"/>
  <Default Extension="vsixmanifest" ContentType="text/xml"/>
</Types>"""
with zipfile.ZipFile(Path(sys.argv[1]), "x", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", types)
    z.writestr("extension.vsixmanifest", vsix)
    z.writestr("extension/package.json", json.dumps(manifest))
    z.writestr("extension/extension.js", """
const vscode = require('vscode');
const fs = require('fs');
const dir = '/root/nexus-workbench';
const path = dir + '/nexus-editor-fixture.txt';
exports.activate = function (context) {
    fs.writeFileSync(dir + '/nexus-extension-activated.marker', 'activated');
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(doc => {
        if (doc.uri.fsPath === path && doc.getText() === 'NEXUS_BROWSER_EDITOR_SAVED') {
            fs.writeFileSync(dir + '/nexus-editor-saved.marker', 'saved');
        }
    }));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.fixture.open', async () => {
        const uri = vscode.Uri.file(path);
        await vscode.workspace.fs.writeFile(uri, Buffer.from('NEXUS_BROWSER_EDITOR_INITIAL'));
        await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    }));
};
exports.deactivate = function () {};
""")
