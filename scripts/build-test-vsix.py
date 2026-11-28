"""Generate a local CI-only VSIX and stdio LSP peer. No marketplace, dependencies or network."""
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
    z.writestr("extension/extension.js", r"""
const vscode = require('vscode');
const fs = require('fs');
const { spawn } = require('child_process');
const nodePath = require('path');
const dir = '/root/nexus-workbench';
const path = dir + '/nexus-editor-fixture.txt';
async function complete(doc, pos) {
    const child = spawn(process.execPath, ['--jitless', nodePath.join(__dirname, 'server.js')], {
        stdio: ['pipe', 'pipe', 'ignore'], cwd: dir, env: { PATH: '/usr/bin:/bin', HOME: '/root' }
    });
    let buffer = Buffer.alloc(0), pending = null, failed = false;
    const closed = new Promise(resolve => child.once('close', resolve));
    function reject(error) { failed = true; if (pending) { const p = pending; pending = null; p.reject(error); } }
    child.on('error', reject);
    child.stdin.on('error', reject);
    child.once('close', () => reject(new Error('peer closed')));
    child.stdout.on('data', data => {
        try {
            buffer = Buffer.concat([buffer, data]);
            if (buffer.length > 32768) throw Error('response limit');
            while (true) {
                const end = buffer.indexOf('\r\n\r\n');
                if (end < 0) return;
                const match = /^Content-Length: ([0-9]+)$/i.exec(buffer.subarray(0, end).toString());
                if (!match || Number(match[1]) > 16384) throw Error('header');
                const n = Number(match[1]), start = end + 4;
                if (buffer.length < start + n) return;
                const reply = JSON.parse(buffer.subarray(start, start+n).toString());
                buffer = buffer.subarray(start+n);
                if (!pending || reply.id !== pending.id || reply.jsonrpc !== '2.0' || reply.error) throw Error('reply');
                const p = pending; pending = null; p.resolve(reply.result);
            }
        } catch (err) { reject(err); child.kill(); }
    });
    function send(message) {
        const bytes = Buffer.from(JSON.stringify({jsonrpc:'2.0', ...message}));
        child.stdin.write(Buffer.concat([Buffer.from('Content-Length: '+bytes.length+'\r\n\r\n'), bytes]));
    }
    function call(id, method, params) {
        if (failed || pending) return Promise.reject(Error('peer unavailable'));
        return new Promise((resolve,reject) => { pending={id,resolve,reject}; send({id,method,params}); });
    }
    let timer;
    try {
        const work = (async () => {
            const init = await call(1, 'initialize', {processId:null,rootUri:null,capabilities:{}});
            if (!init.capabilities.completionProvider) throw Error('capability');
            send({method:'initialized',params:{}});
            send({method:'textDocument/didOpen',params:{textDocument:{uri:doc.uri.toString(),languageId:doc.languageId,version:doc.version,text:doc.getText()}}});
            const items = await call(2,'textDocument/completion',{textDocument:{uri:doc.uri.toString()},position:{line:pos.line,character:pos.character}});
            if (!Array.isArray(items) || items.length!==1 || items[0].label!=='NEXUS_LSP_COMPLETION') throw Error('completion');
            await call(3,'shutdown',null);
            send({method:'exit'}); child.stdin.end();
            if (await closed !== 0) throw Error('exit');
            const item = new vscode.CompletionItem(items[0].label);
            item.insertText = items[0].insertText;
            item.range = new vscode.Range(pos,pos);
            return [item];
        })();
        return await Promise.race([work,new Promise((_,reject) => { timer=setTimeout(()=>reject(Error('timeout')),15000); })]);
    } finally { clearTimeout(timer); if (child.exitCode===null) child.kill(); }
}
exports.activate = function (context) {
    fs.writeFileSync(dir + '/nexus-extension-activated.marker', 'activated');
    context.subscriptions.push(vscode.languages.registerCompletionItemProvider({scheme:'file',language:'plaintext'}, {
        provideCompletionItems(doc,pos) {
            if (doc.uri.fsPath !== path || pos.line!==1 || pos.character!==0 || doc.getText()!=='NEXUS_BROWSER_EDITOR_SAVED\n') return [];
            return complete(doc,pos);
        }
    }));
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(doc => {
        if (doc.uri.fsPath === path && doc.getText() === 'NEXUS_BROWSER_EDITOR_SAVED') {
            fs.writeFileSync(dir + '/nexus-editor-saved.marker', 'saved');
        }
        if (doc.uri.fsPath === path && doc.getText() === 'NEXUS_BROWSER_EDITOR_SAVED\nNEXUS_LSP_COMPLETION') {
            fs.writeFileSync(dir + '/nexus-lsp-saved.marker', 'saved');
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
    z.writestr("extension/server.js", r"""
const fs = require('fs');
const dir='/root/nexus-workbench';
let buffer=Buffer.alloc(0), initialized=false, opened=false, completed=false, shuttingDown=false;
function reply(id,result) {
    const data=Buffer.from(JSON.stringify({jsonrpc:'2.0',id,result}));
    process.stdout.write(Buffer.concat([Buffer.from('Content-Length: '+data.length+'\r\n\r\n'),data]));
}
function handle(msg) {
    if (msg.jsonrpc!=='2.0') throw Error('protocol');
    if (msg.method==='initialize'&&msg.id===1&&!initialized) {
        initialized=true; reply(1,{capabilities:{completionProvider:{},textDocumentSync:1}});
    } else if (msg.method==='initialized'&&initialized&&!opened) {
        return;
    } else if (msg.method==='textDocument/didOpen'&&initialized&&!opened) {
        const doc=msg.params.textDocument;
        if (doc.uri!=='file:///root/nexus-workbench/nexus-editor-fixture.txt'||doc.text!=='NEXUS_BROWSER_EDITOR_SAVED\n') throw Error('document');
        opened=true;
    } else if (msg.method==='textDocument/completion'&&msg.id===2&&opened&&!completed) {
        const p=msg.params;
        if (p.textDocument.uri!=='file:///root/nexus-workbench/nexus-editor-fixture.txt'||p.position.line!==1||p.position.character!==0) throw Error('position');
        completed=true;
        reply(2,[{label:'NEXUS_LSP_COMPLETION',insertText:'NEXUS_LSP_COMPLETION'}]);
    } else if (msg.method==='shutdown'&&msg.id===3&&completed&&!shuttingDown) {
        shuttingDown=true; reply(3,null);
    } else if (msg.method==='exit'&&shuttingDown) {
        fs.writeFileSync(dir+'/nexus-lsp-transport.marker','completed'); process.exit(0);
    } else throw Error('unexpected request');
}
process.stdin.on('data',chunk=>{
    try {
        buffer=Buffer.concat([buffer,chunk]);
        if(buffer.length>32768)throw Error('limit');
        while(true){
            const end=buffer.indexOf('\r\n\r\n');if(end<0)return;
            const match=/^Content-Length: ([0-9]+)$/i.exec(buffer.subarray(0,end).toString());
            if(!match||Number(match[1])>16384)throw Error('header');
            const n=Number(match[1]),start=end+4;if(buffer.length<start+n)return;
            const frame=JSON.parse(buffer.subarray(start,start+n).toString());buffer=buffer.subarray(start+n);
            handle(frame);
        }
    } catch (_) { process.exit(1); }
});
process.stdin.on('end',()=>process.exit(1));
""")
