'use strict';
// Fixed, reviewable acceptance extension. Real pylsp subprocess, not an LSP peer.
const vscode = require('vscode');
const fs = require('fs');
const { spawn } = require('child_process');
const dir = '/root/nexus-workbench';
const path = dir + '/nexus-real-lsp.py';
const initial = 'missing_nexus_symbol\nclass NexusCompletion:\n    nexus_value = 42\nitem = NexusCompletion()\nitem';
const final = initial + '.nexus_value';
let completed = null;
async function request(doc, operation, pos) {
    const uri = doc.uri.toString(), text = doc.getText(), version = doc.version;
    const child = spawn('/usr/bin/python3', ['-u','-m','pylsp'], {
        cwd: dir, stdio: ['pipe','pipe','ignore'],
        env: {PATH:'/usr/bin:/bin', HOME:'/root', LANG:'C.UTF-8', PYTHONFAULTHANDLER:'1'}
    });
    let buffer = Buffer.alloc(0), pending = null, error = null, diagnostic = null, notify = null;
    const closed = new Promise(resolve => child.once('close', resolve));
    function fail(e) {
        error = e;
        if (pending) { const p=pending; pending=null; p.reject(e); }
        if (notify) { const n=notify; notify=null; n.reject(e); }
    }
    child.on('error', fail); child.stdin.on('error', fail);
    child.once('close', () => fail(Error('pylsp closed')));
    child.stdout.on('data', data => {
        try {
            buffer=Buffer.concat([buffer,data]);
            if (buffer.length>1048576) throw Error('LSP buffer limit');
            while (true) {
                const end=buffer.indexOf('\r\n\r\n'); if(end<0) return;
                const match=/^Content-Length: ([0-9]+)(?:\r\nContent-Type: [^\r\n]+)?$/i.exec(buffer.subarray(0,end).toString());
                if(!match || +match[1]>524288) throw Error('LSP header');
                const n=+match[1], start=end+4; if(buffer.length<start+n) return;
                const r=JSON.parse(buffer.subarray(start,start+n).toString());
                buffer=buffer.subarray(start+n);
                if(r.jsonrpc!=='2.0') throw Error('protocol');
                if(r.method==='textDocument/publishDiagnostics' && r.params.uri===uri) {
                    if(r.params.version!==undefined && r.params.version!==version) throw Error('stale diagnostic');
                    if(!Array.isArray(r.params.diagnostics)) throw Error('diagnostics');
                    if(r.params.diagnostics.some(d=>typeof d.message==='string'&&d.message.includes('missing_nexus_symbol')&&d.range.start.line===0&&d.range.start.character===0)){
                        diagnostic=r.params.diagnostics;
                        if(notify){const n=notify;notify=null;n.resolve(diagnostic);}
                    }
                } else if(r.method && r.id!==undefined) {
                    // No server-initiated file access, commands or dynamic registration.
                    send({id:r.id,error:{code:-32601,message:'Not supported'}});
                } else if(r.method) {
                    continue; // server log/progress notifications
                } else {
                    if(!pending || r.id!==pending.id || r.error) throw Error('LSP reply');
                    const p=pending;pending=null;p.resolve(r.result);
                }
            }
        } catch(e){fail(e);child.kill();}
    });
    function send(msg) {
        const bytes=Buffer.from(JSON.stringify({jsonrpc:'2.0',...msg}));
        child.stdin.write(Buffer.concat([Buffer.from('Content-Length: '+bytes.length+'\r\n\r\n'),bytes]));
    }
    function call(id,method,params){
        if(error||pending) return Promise.reject(error||Error('busy'));
        return new Promise((resolve,reject)=>{pending={id,resolve,reject};send({id,method,params});});
    }
    let timer;
    try {
        const work=(async()=>{
            const init=await call(1,'initialize',{processId:null,rootUri:'file:///root/nexus-workbench',capabilities:{textDocument:{completion:{completionItem:{snippetSupport:false}},publishDiagnostics:{versionSupport:true}}}});
            if(init.serverInfo?.name!=='pylsp'||init.serverInfo?.version!=='1.10.0'||!init.capabilities.completionProvider) throw Error('real server identity');
            send({method:'initialized',params:{}});
            send({method:'textDocument/didOpen',params:{textDocument:{uri,languageId:'python',version,text}}});
            let result;
            if(operation==='diagnostic'){
                result=diagnostic||await new Promise((resolve,reject)=>{notify={resolve,reject};});
            }else{
                const reply=await call(2,'textDocument/completion',{textDocument:{uri},position:{line:pos.line,character:pos.character},context:{triggerKind:1}});
                const items=Array.isArray(reply)?reply:reply?.items;
                result=items?.find(i=>i.label==='nexus_value'&&i.insertText==='nexus_value'&&!i.textEdit);
                if(!result) throw Error('missing real Jedi completion');
            }
            send({method:'textDocument/didClose',params:{textDocument:{uri}}});
            await call(3,'shutdown',null);
            send({method:'exit'});child.stdin.end();
            if(await closed!==0) throw Error('pylsp exit');
            if(doc.version!==version||doc.getText()!==text) throw Error('editor changed');
            return result;
        })();
        return await Promise.race([work,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('pylsp timeout')),120000);})]);
    } finally {clearTimeout(timer);if(child.exitCode===null&&child.signalCode===null)child.kill();}
}
exports.activate=function(context){
    const diagnostics=vscode.languages.createDiagnosticCollection('nexus-real-pylsp');
    context.subscriptions.push(diagnostics);
    fs.writeFileSync(dir+'/nexus-extension-activated.marker','activated');
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(async doc=>{
        if(doc.uri.fsPath!==path)return;
        if(doc.getText()===initial){
            try{
                const rows=await request(doc,'diagnostic');
                const selected=rows.filter(d=>d.message.includes('missing_nexus_symbol')&&d.range.start.line===0&&d.range.start.character===0);
                const projected=selected.map(d=>new vscode.Diagnostic(new vscode.Range(d.range.start.line,d.range.start.character,d.range.end.line,d.range.end.character),d.message,vscode.DiagnosticSeverity.Error));
                diagnostics.set(doc.uri,projected);
                fs.writeFileSync(dir+'/nexus-editor-saved.marker','saved');
                fs.writeFileSync(dir+'/nexus-real-diagnostics.marker','pylsp-1.10.0');
            }catch(_){vscode.window.showWarningMessage('Real pylsp diagnostics failed');}
        }else if(doc.getText()===final&&completed){
            fs.writeFileSync(dir+'/nexus-lsp-saved.marker','saved');
        }
    }));
    let running;
    context.subscriptions.push(vscode.languages.registerCompletionItemProvider({scheme:'file',language:'python'},{
        async provideCompletionItems(doc,pos){
            if(doc.uri.fsPath!==path||doc.getText()!==initial+'.'||pos.line!==4||pos.character!==5)return [];
            if(!running)running=request(doc,'completion',pos).finally(()=>{running=null;});
            try{
                const returned=await running;
                const item=new vscode.CompletionItem(returned.label);
                item.insertText=returned.insertText;
                item.range=new vscode.Range(pos,pos);
                completed=true;
                fs.writeFileSync(dir+'/nexus-lsp-transport.marker','completed');
                fs.writeFileSync(dir+'/nexus-real-completion.marker','pylsp-1.10.0');
                return [item];
            }catch(_){return [];}
        }
    }));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.fixture.open',async()=>{
        const uri=vscode.Uri.file(path);
        await vscode.workspace.fs.writeFile(uri,Buffer.from('# Nexus real LSP acceptance'));
        await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    }));
};
exports.deactivate=function(){};
