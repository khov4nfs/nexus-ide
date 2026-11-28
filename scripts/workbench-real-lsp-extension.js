'use strict';
// Fixed, reviewable acceptance extension. Real pylsp subprocess, not an LSP peer.
const vscode = require('vscode');
const fs = require('fs');
const { spawn } = require('child_process');
const nodePath = require('path');
const dir = '/root/nexus-workbench';
const path = dir + '/nexus-real-lsp.py';
const initial = 'missing_nexus_symbol\nclass NexusCompletion:\n    nexus_value = 42\nitem = NexusCompletion()\nitem';
const final = initial + '.nexus_value';
let completed = false, acceptance = false;
let tail = Promise.resolve(), queued = 0;
function supported(doc) {
    return vscode.workspace.isTrusted && doc.uri.scheme==='file' && doc.languageId==='python' &&
        nodePath.resolve(doc.uri.fsPath).startsWith(dir+'/') && Buffer.byteLength(doc.getText(),'utf8')<=131072;
}
function serialize(work) {
    if(queued>=3)return Promise.reject(Error('LSP queue full'));
    queued++;
    const task=tail.then(work);
    tail=task.catch(()=>{}).finally(()=>{queued--;});
    return task;
}
async function request(doc, operation, pos) {
    if(!supported(doc))throw Error('unsupported document');
    const uri = doc.uri.toString(), text = doc.getText(), version = doc.version;
    const test = acceptance && doc.uri.fsPath===path;
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
                    if(!test || operation!=='diagnostic' || r.params.diagnostics.some(d=>typeof d.message==='string'&&d.message.includes('missing_nexus_symbol')&&d.range?.start?.line===0&&d.range.start.character===0)){
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
                if(!Array.isArray(items))throw Error('invalid completion response');
                // Narrow initial projection: plain identifiers only; no snippets,
                // server commands, extra edits or arbitrary textEdit application.
                result=items.filter(i=>typeof i.label==='string'&&typeof i.insertText==='string'&&i.label.length<=160&&/^[A-Za-z_][A-Za-z0-9_]*$/.test(i.insertText)&&!i.textEdit&&!i.additionalTextEdits&&!i.command&&(i.insertTextFormat===undefined||i.insertTextFormat===1)).slice(0,100);
                if(test && !result.some(i=>i.label==='nexus_value'&&i.insertText==='nexus_value'))throw Error('missing real Jedi completion');
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
    function boundedPosition(doc,p){
        return p && Number.isInteger(p.line)&&Number.isInteger(p.character)&&p.line>=0&&p.line<doc.lineCount&&p.character>=0&&p.character<=doc.lineAt(p.line).text.length;
    }
    function project(doc,rows){
        return rows.slice(0,200).flatMap(d=>{
            const a=d.range?.start,b=d.range?.end;
            if(!boundedPosition(doc,a)||!boundedPosition(doc,b)||a.line>b.line||(a.line===b.line&&a.character>b.character)||typeof d.message!=='string')return [];
            const severity=({1:vscode.DiagnosticSeverity.Error,2:vscode.DiagnosticSeverity.Warning,3:vscode.DiagnosticSeverity.Information,4:vscode.DiagnosticSeverity.Hint})[d.severity]??vscode.DiagnosticSeverity.Warning;
            const item=new vscode.Diagnostic(new vscode.Range(a.line,a.character,b.line,b.character),d.message.slice(0,2048),severity);
            item.source='pylsp';
            return [item];
        });
    }
    context.subscriptions.push(vscode.workspace.onDidChangeTextDocument(e=>diagnostics.delete(e.document.uri)));
    context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(doc=>diagnostics.delete(doc.uri)));
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(async doc=>{
        if(!supported(doc))return;
        const text=doc.getText(), version=doc.version;
        const test=acceptance&&doc.uri.fsPath===path;
        if(test&&text===final&&completed){
            fs.writeFileSync(dir+'/nexus-lsp-saved.marker','saved');
            return;
        }
        try{
            const rows=await serialize(()=>{
                if(doc.version!==version||doc.getText()!==text)throw Error('snapshot changed');
                return request(doc,'diagnostic');
            });
            if(doc.version!==version||doc.isClosed)return;
            const projected=project(doc,rows);
            diagnostics.set(doc.uri,projected);
            if(test&&text===initial&&projected.some(d=>d.message.includes('missing_nexus_symbol')&&d.range.start.line===0&&d.range.start.character===0)){
                fs.writeFileSync(dir+'/nexus-editor-saved.marker','saved');
                fs.writeFileSync(dir+'/nexus-real-diagnostics.marker','pylsp-1.10.0');
            }
        }catch(_){if(test)vscode.window.showWarningMessage('Real pylsp diagnostics failed');}
    }));
    let running=null, runningKey='';
    context.subscriptions.push(vscode.languages.registerCompletionItemProvider({scheme:'file',language:'python'},{
        async provideCompletionItems(doc,pos,token){
            if(!supported(doc)||token.isCancellationRequested)return [];
            const text=doc.getText(), version=doc.version, key=doc.uri.toString()+':'+version+':'+pos.line+':'+pos.character;
            const test=acceptance&&doc.uri.fsPath===path;
            if(test&&(text!==initial+'.'||pos.line!==4||pos.character!==5))return [];
            if(running&&runningKey!==key)return [];
            if(!running){
                runningKey=key;
                running=serialize(()=>{
                    if(doc.version!==version||doc.getText()!==text)throw Error('snapshot changed');
                    return request(doc,'completion',pos);
                }).finally(()=>{running=null;runningKey='';});
            }
            try{
                const returned=await running;
                if(doc.version!==version||doc.isClosed||token.isCancellationRequested)return [];
                const word=doc.getWordRangeAtPosition(pos,/[A-Za-z_][A-Za-z0-9_]*/);
                const range=word&&word.start.isBeforeOrEqual(pos)&&word.end.isEqual(pos)?new vscode.Range(word.start,pos):new vscode.Range(pos,pos);
                const items=returned.map(row=>{
                    const item=new vscode.CompletionItem(row.label);
                    item.insertText=row.insertText;
                    item.range=range;
                    return item;
                });
                if(test&&items.some(i=>i.label==='nexus_value'&&i.insertText==='nexus_value')){
                    completed=true;
                    fs.writeFileSync(dir+'/nexus-lsp-transport.marker','completed');
                    fs.writeFileSync(dir+'/nexus-real-completion.marker','pylsp-1.10.0');
                }
                return test?items.filter(i=>i.label==='nexus_value'):items;
            }catch(_){return [];}
        }
    }));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.fixture.open',async()=>{
        if(!vscode.workspace.isTrusted){vscode.window.showWarningMessage('Trust the reviewed guest workspace first.');return;}
        if(fs.existsSync(path)){vscode.window.showWarningMessage('Acceptance file already exists. Use a fresh reviewed guest or remove that test file yourself.');return;}
        acceptance=true;
        completed=false;
        fs.writeFileSync(dir+'/nexus-extension-activated.marker','activated');
        const uri=vscode.Uri.file(path);
        fs.writeFileSync(path,'# Nexus real LSP acceptance',{flag:'wx'});
        await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    }));
};
exports.deactivate=function(){};
