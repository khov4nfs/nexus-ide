'use strict';
// Reviewed guest-local persistent language support. No automatic downloads.
const vscode=require('vscode');
const fs=require('fs');
const path=require('path');
const {Session,profileFor,ROOT}=require('./lsp-session.js');
const fixtures={
    python:{filename:'nexus-real-lsp.py',initial:'missing_nexus_symbol\nclass NexusCompletion:\n    nexus_value = 42\nitem = NexusCompletion()\nitem',suffix:'.nexus_value',symbol:'missing_nexus_symbol',line:0,character:0,label:'nexus_value',completionLine:4,completionCharacter:5,diagnosticMarker:'nexus-real-diagnostics.marker',completionMarker:'nexus-real-completion.marker',savedMarker:'nexus-lsp-saved.marker',server:'pylsp-1.10.0'},
    c:{filename:'nexus-real-lsp.c',initial:'int broken = missing_nexus_c;\nstruct NexusC { int nexus_c_value; };\nvoid probe_c(void) {\n    struct NexusC item;\n    item',suffix:'.nexus_c_value',symbol:'missing_nexus_c',line:0,character:13,label:'nexus_c_value',completionLine:4,completionCharacter:9,diagnosticMarker:'nexus-c-diagnostics.marker',completionMarker:'nexus-c-completion.marker',savedMarker:'nexus-c-saved.marker',server:'clangd-18.1.3'},
    cpp:{filename:'nexus-real-lsp.cpp',initial:'int broken = missing_nexus_cpp;\nstruct NexusCompletion { int nexus_value; };\nvoid probe() {\n    NexusCompletion item;\n    item',suffix:'.nexus_value',symbol:'missing_nexus_cpp',line:0,character:13,label:'nexus_value',completionLine:4,completionCharacter:9,diagnosticMarker:'nexus-cpp-diagnostics.marker',completionMarker:'nexus-cpp-completion.marker',savedMarker:'nexus-cpp-saved.marker',server:'clangd-18.1.3'}
};
const sessions=new Map(),accepted=new Set(),completed=new Set();
let stopping=false, collection;
function marker(name,value){fs.writeFileSync(path.join(ROOT,name),value);}
function supported(doc){
    return !stopping&&vscode.workspace.isTrusted&&doc.uri.scheme==='file'&&profileFor(doc.languageId)&&
        path.resolve(doc.uri.fsPath).startsWith(ROOT+'/')&&Buffer.byteLength(doc.getText(),'utf8')<=131072;
}
function snapshot(doc){return {uri:doc.uri.toString(),language:doc.languageId,version:doc.version,text:doc.getText()};}
function session(language){
    const key=profileFor(language);
    let s=sessions.get(key);
    if(!s||s.failure){
        // No automatic restart loop. A new explicit editor operation may
        // recover a failed process; acceptance counters preserve failures.
        s=new Session(key);
        const previous=sessions.get(key);
        if(previous)s.restarted=true;
        sessions.set(key,s);
    }
    return s;
}
function fixture(doc){
    const f=fixtures[doc.languageId];
    return f&&accepted.has(f.filename)&&doc.uri.fsPath===path.join(ROOT,f.filename)?f:null;
}
function validPos(doc,p){
    return p&&Number.isInteger(p.line)&&Number.isInteger(p.character)&&p.line>=0&&p.line<doc.lineCount&&p.character>=0&&p.character<=doc.lineAt(p.line).text.length;
}
function project(doc,rows,source){
    return rows.slice(0,200).flatMap(d=>{
        const a=d.range?.start,b=d.range?.end;
        if(!validPos(doc,a)||!validPos(doc,b)||a.line>b.line||(a.line===b.line&&a.character>b.character)||typeof d.message!=='string')return [];
        const severity=({1:vscode.DiagnosticSeverity.Error,2:vscode.DiagnosticSeverity.Warning,3:vscode.DiagnosticSeverity.Information,4:vscode.DiagnosticSeverity.Hint})[d.severity]??vscode.DiagnosticSeverity.Warning;
        const item=new vscode.Diagnostic(new vscode.Range(a.line,a.character,b.line,b.character),d.message.slice(0,2048),severity);
        item.source=source;
        return [item];
    });
}
async function diagnostics(doc){
    if(!supported(doc))return;
    const snap=snapshot(doc),f=fixture(doc);
    const server=session(doc.languageId);
    try{
        const rows=await server.diagnostic(snap);
        if(doc.version!==snap.version||doc.isClosed||stopping)return;
        const projected=project(doc,rows,server.profile.name);
        collection.set(doc.uri,projected);
        if(f&&snap.text===f.initial&&projected.some(d=>d.message.includes(f.symbol)&&d.range.start.line===f.line&&d.range.start.character===f.character)){
            if(doc.languageId==='python')marker('nexus-editor-saved.marker','saved');
            marker(f.diagnosticMarker,f.server);
        }
    }catch(_){if(f&&!stopping)vscode.window.showWarningMessage('Real language server diagnostics failed');}
}
function completionItem(doc,pos,row){
    if(!row||typeof row.label!=='string'||row.label.length>160||row.command||row.additionalTextEdits||
       (row.insertTextFormat!==undefined&&row.insertTextFormat!==1))return null;
    let text=row.insertText??row.label.trim(),range;
    if(row.textEdit){
        const edit=row.textEdit;
        const a=edit.range?.start,b=edit.range?.end;
        if(!validPos(doc,a)||!validPos(doc,b)||a.line!==pos.line||b.line!==pos.line||
            a.character>pos.character||b.character!==pos.character)return null;
        text=edit.newText;range=new vscode.Range(a.line,a.character,b.line,b.character);
    }else{
        const word=doc.getWordRangeAtPosition(pos,/[A-Za-z_][A-Za-z0-9_]*/);
        range=word&&word.start.isBeforeOrEqual(pos)&&word.end.isEqual(pos)?new vscode.Range(word.start,pos):new vscode.Range(pos,pos);
    }
    if(typeof text!=='string'||!/^[A-Za-z_][A-Za-z0-9_]*$/.test(text))return null;
    const item=new vscode.CompletionItem(row.label.trim());
    item.insertText=text;item.range=range;
    return item;
}
async function stopAll(writeEvidence=false){
    stopping=true;
    const reports={};
    await Promise.all([...sessions].map(async ([key,s])=>{
        reports[key]={...await s.stop(),restarted:!!s.restarted};
    }));
    collection?.clear();
    if(writeEvidence&&Object.values(fixtures).every(f=>accepted.has(f.filename))){
        const p=reports.python,c=reports.clangd;
        // Evidence must reflect actual initialized server processes, not just
        // successful marker writes or completion text.
        const ok=[p,c].every(s=>s&&s.launches===1&&s.initializes===1&&s.diagnostics>=1&&s.completions>=1&&s.shutdownClean&&!s.restarted&&s.opens>=1&&s.closes===s.opens)&&
            p.snapshotRefreshes>=1&&c.changes>=2&&c.completions>=2&&c.peakDocuments>=2&&Object.values(fixtures).every(f=>completed.has(f.filename));
        marker('nexus-persistent-lsp.json',JSON.stringify({ok,python:p,clangd:c,languages:['python','c','cpp']}));
    }
}
exports.activate=function(context){
    collection=vscode.languages.createDiagnosticCollection('nexus-reviewed-lsp');
    context.subscriptions.push(collection);
    const scheduled=new Map();
    // On-save diagnostics and explicit completion are bounded; a stale
    // result never reaches a newer editor snapshot. Clear on every edit.
    context.subscriptions.push(vscode.workspace.onDidChangeTextDocument(e=>collection.delete(e.document.uri)));
    context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(doc=>{
        collection.delete(doc.uri);
        const s=sessions.get(profileFor(doc.languageId));if(s)s.closeDocument(doc.uri.toString()).catch(()=>{});
    }));
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(doc=>{
        if(!supported(doc))return;
        const f=fixture(doc),text=doc.getText();
        if(f&&text===f.initial+f.suffix&&completed.has(f.filename)){
            marker(f.savedMarker,'saved');return;
        }
        const key=doc.uri.toString();
        if(scheduled.has(key))clearTimeout(scheduled.get(key));
        scheduled.set(key,setTimeout(()=>{scheduled.delete(key);diagnostics(doc);},150));
    }));
    context.subscriptions.push({dispose(){for(const timer of scheduled.values())clearTimeout(timer);scheduled.clear();}});
    for(const language of ['python','c','cpp']){
        context.subscriptions.push(vscode.languages.registerCompletionItemProvider({scheme:'file',language},{
            async provideCompletionItems(doc,pos,token){
                if(!supported(doc)||token.isCancellationRequested)return [];
                const f=fixture(doc),snap=snapshot(doc);
                if(f&&(snap.text!==f.initial+'.'||pos.line!==f.completionLine||pos.character!==f.completionCharacter))return [];
                try{
                    const rows=await session(language).complete(snap,{line:pos.line,character:pos.character},token);
                    if(doc.version!==snap.version||doc.isClosed||token.isCancellationRequested||stopping)return [];
                    let items=rows.map(r=>completionItem(doc,pos,r)).filter(Boolean).slice(0,100);
                    if(f){
                        items=items.filter(i=>i.label===f.label&&i.insertText===f.label);
                        if(items.length){
                            completed.add(f.filename);marker(f.completionMarker,f.server);
                            if(language==='python')marker('nexus-lsp-transport.marker','completed');
                        }
                    }
                    return items;
                }catch(_){return [];}
            }
        }));
    }
    async function openFixture(f){
        if(!vscode.workspace.isTrusted||stopping)return;
        const filename=path.join(ROOT,f.filename);
        if(fs.existsSync(filename)){vscode.window.showWarningMessage('Acceptance file exists. Use fresh reviewed guest.');return;}
        fs.writeFileSync(filename,'// Nexus real LSP acceptance',{flag:'wx'});
        accepted.add(f.filename);completed.delete(f.filename);
        if(f===fixtures.python)marker('nexus-extension-activated.marker','activated');
        await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(vscode.Uri.file(filename)));
    }
    context.subscriptions.push(vscode.commands.registerCommand('nexus.fixture.open',()=>openFixture(fixtures.python)));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.fixture.cpp',()=>openFixture(fixtures.cpp)));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.fixture.c',()=>openFixture(fixtures.c)));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.stop',()=>stopAll(true)));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.restart',async()=>{
        await stopAll(false);sessions.clear();stopping=false;
    }));
};
exports.deactivate=()=>stopAll(false);
