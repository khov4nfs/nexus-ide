'use strict';
// Reviewed guest-local persistent language support. No automatic downloads.
const vscode=require('vscode');
const fs=require('fs');
const path=require('path');
const {Session,profileFor,PROFILES,LANGUAGES,TOOL_ROOT,ROOT}=require('./lsp-session.js');
const {checkedPath,plan}=require('./lsp-edits.js');
const fixtures={
    python:{filename:'nexus-real-lsp.py',initial:'missing_nexus_symbol\nclass NexusCompletion:\n    nexus_value = 42\nitem = NexusCompletion()\nitem',suffix:'.nexus_value',symbol:'missing_nexus_symbol',line:0,character:0,label:'nexus_value',completionLine:4,completionCharacter:5,diagnosticMarker:'nexus-real-diagnostics.marker',completionMarker:'nexus-real-completion.marker',savedMarker:'nexus-lsp-saved.marker',server:'pylsp-1.10.0'},
    c:{filename:'nexus-real-lsp.c',initial:'int broken = missing_nexus_c;\nstruct NexusC { int nexus_c_value; };\nvoid probe_c(void) {\n    struct NexusC item;\n    item',suffix:'.nexus_c_value',symbol:'missing_nexus_c',line:0,character:13,label:'nexus_c_value',completionLine:4,completionCharacter:9,diagnosticMarker:'nexus-c-diagnostics.marker',completionMarker:'nexus-c-completion.marker',savedMarker:'nexus-c-saved.marker',server:'clangd-18.1.3'},
    cpp:{filename:'nexus-real-lsp.cpp',initial:'int broken = missing_nexus_cpp;\nstruct NexusCompletion { int nexus_value; };\nvoid probe() {\n    NexusCompletion item;\n    item',suffix:'.nexus_value',symbol:'missing_nexus_cpp',line:0,character:13,label:'nexus_value',completionLine:4,completionCharacter:9,diagnosticMarker:'nexus-cpp-diagnostics.marker',completionMarker:'nexus-cpp-completion.marker',savedMarker:'nexus-cpp-saved.marker',server:'clangd-18.1.3'}
};
const sessions=new Map(),accepted=new Set(),completed=new Set(),approvals=new Map();
let projectIndex=false,previewText='',projectBusy=false,projectAcceptance=false;
let stopping=false, collection, stopInProgress=null;
function marker(name,value){fs.writeFileSync(path.join(ROOT,name),value);}
function supported(doc){
    return !stopping&&!doc.isClosed&&vscode.workspace.isTrusted&&doc.uri.scheme==='file'&&profileFor(doc.languageId)&&
        path.resolve(doc.uri.fsPath).startsWith(ROOT+'/')&&Buffer.byteLength(doc.getText(),'utf8')<=131072&&
        (!PROFILES[profileFor(doc.languageId)].optional||approvals.has(profileFor(doc.languageId)));
}
function snapshot(doc){return {uri:doc.uri.toString(),language:doc.languageId,version:doc.version,text:doc.getText()};}
function session(language){
    const key=profileFor(language);
    let s=sessions.get(key);
    if(!s){
        // Failed servers stay failed until the user explicitly restarts.
        // A live document edit must not create a crash/restart loop.
        if(sessions.size>=4)throw Error('Stop language servers before starting more profiles');
        s=new Session(key,{approval:approvals.get(key),projectIndex});
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
    try{
        const server=session(doc.languageId);
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
    if(stopInProgress)return stopInProgress;
    stopInProgress=(async()=>{
    const reports={};
    await Promise.all([...sessions].map(async ([key,s])=>{
        reports[key]={...await s.stop(),restarted:!!s.restarted};
    }));
    collection?.clear();
    if(writeEvidence&&Object.values(fixtures).every(f=>accepted.has(f.filename))){
        const p=reports.python,c=reports.clangd;
        // Evidence must reflect actual initialized server processes, not just
        // successful marker writes or completion text.
        const ok=projectAcceptance&&[p,c].every(s=>s&&s.launches===1&&s.initializes===1&&s.diagnostics>=1&&s.completions>=1&&s.shutdownClean&&s.processExited&&!s.restarted&&s.opens>=1&&s.closes===s.opens)&&
            p.snapshotRefreshes>=1&&c.changes>=2&&c.completions>=2&&c.peakDocuments>=2&&Object.values(fixtures).every(f=>completed.has(f.filename));
        marker('nexus-persistent-lsp.json',JSON.stringify({ok,python:p,clangd:c,languages:['python','c','cpp'],projectAcceptance}));
    }
    return Object.values(reports).every(r=>r.launches===0||r.processExited===true);
    })();
    try{return await stopInProgress;}finally{stopInProgress=null;}
}
function existingProjectFile(uri){
    const file=checkedPath(uri,ROOT);
    // Real path equality rejects all symlink components. Project data is
    // untrusted even after a workspace trust grant; no external paths.
    if(fs.realpathSync(file)!==file||!fs.statSync(file).isFile())throw Error('unsafe project file');
    return file;
}
function currentOpen(uri){return vscode.workspace.textDocuments.find(d=>d.uri.toString()===uri&&!d.isClosed);}
function boundedFile(uri){
    const file=existingProjectFile(uri);
    if(fs.statSync(file).size>131072)throw Error('file too large');
    const raw=fs.readFileSync(file),text=raw.toString('utf8');
    if(!Buffer.from(text,'utf8').equals(raw)||text.includes('\0'))throw Error('non-text file');
    return text;
}
function currentBaseline(uri){
    const disk=boundedFile(uri),doc=currentOpen(uri);
    if(doc){
        if(Buffer.byteLength(doc.getText())>131072||doc.getText().includes('\0'))throw Error('invalid open file');
        return {text:doc.getText(),version:doc.version,disk};
    }
    return {text:disk,version:null,disk};
}
async function captureBaselines(){
    // The bounded allowlist includes only source files, not credentials,
    // dependency trees, generated output, or hidden files. Fail closed for
    // larger workspaces rather than silently truncate a rename.
    const includes=new vscode.RelativePattern(ROOT,'**/*.{py,c,cc,cpp,cxx,h,hh,hpp,hxx,ts,tsx,js,jsx,mjs,cjs,go,rs,sh,bash}');
    const excludes='**/{.*,node_modules,target,vendor,build,dist}/**';
    const uris=await vscode.workspace.findFiles(includes,excludes,129);
    if(uris.length>128)throw Error('project baseline exceeds 128 files');
    const baselines=new Map();let total=0;
    for(const u of uris){
        const uri=u.toString();
        const base=currentBaseline(uri);
        if((total+=Buffer.byteLength(base.text)+Buffer.byteLength(base.disk))>2*1024*1024)throw Error('project baseline exceeds 2 MiB');
        baselines.set(uri,base);
    }
    return baselines;
}
function sameBaseline(uri,base){
    try{const now=currentBaseline(uri);return now.version===base.version&&now.text===base.text&&now.disk===base.disk;}catch(_){return false;}
}
function sameOpen(snap){const doc=currentOpen(snap.uri);return !!doc&&doc.version===snap.version&&doc.getText()===snap.text;}
async function projectRPC(doc,method,params,token){
    if(!supported(doc)||fixture(doc))throw Error('project operation unavailable');
    const server=session(doc.languageId);
    if(server.profile.snapshot)throw Error('snapshot server has no safe project operations');
    const key=profileFor(doc.languageId);
    const open=vscode.workspace.textDocuments.filter(d=>supported(d)&&profileFor(d.languageId)===key).map(snapshot);
    if(open.length>12)throw Error('open document limit');
    for(const row of open)existingProjectFile(row.uri);
    const snap=snapshot(doc);
    const result=await server.projectRequest(snap,open,method,params,token);
    if(stopping||!open.every(sameOpen)||!sameOpen(snap)||token?.isCancellationRequested)throw Error('stale project result');
    return {result,server};
}
function location(row){
    try{
        const uri=row?.targetUri??row?.uri, range=row?.targetSelectionRange??row?.range;
        existingProjectFile(uri);
        const a=range?.start,b=range?.end;
        if(![a?.line,a?.character,b?.line,b?.character].every(n=>Number.isSafeInteger(n)&&n>=0&&n<=1000000)||a.line>b.line||(a.line===b.line&&a.character>b.character))return null;
        return new vscode.Location(vscode.Uri.parse(uri),new vscode.Range(a.line,a.character,b.line,b.character));
    }catch(_){return null;}
}
async function reviewProjectEdit(editor,operation){
    if(projectBusy||!editor||!supported(editor.document))return;
    projectBusy=true;
    try{
        const doc=editor.document;
        const pos=editor.selection.active;
        const start=editor.selection.start,end=editor.selection.end;
        const original=snapshot(doc);
        let name;
        if(operation==='rename'){
            name=await vscode.window.showInputBox({prompt:'New symbol name',validateInput:v=>/^[A-Za-z_$][A-Za-z0-9_$]{0,127}$/.test(v)?undefined:'Enter a plain identifier'});
            if(!name)return;
        }
        const baselines=await captureBaselines();
        if(!sameOpen(original)||!baselines.has(original.uri))throw Error('editor changed');
        let response,server;
        if(operation==='rename'){
            ({result:response,server}=await projectRPC(doc,'textDocument/rename',{position:{line:pos.line,character:pos.character},newName:name}));
        }else{
            const range={start:{line:start.line,character:start.character},end:{line:end.line,character:end.character}};
            let actions;
            ({result:actions,server}=await projectRPC(doc,'textDocument/codeAction',{range,context:{diagnostics:[],only:['refactor']}}));
            const choices=Array.isArray(actions)?actions.filter(a=>a?.edit&&!a.command&&!a.disabled&&typeof a.title==='string'&&a.title.length<=160).slice(0,32):[];
            if(!choices.length)throw Error('no text-only refactoring available');
            const chosen=await vscode.window.showQuickPick(choices.map((a,i)=>({label:a.title,index:i})),{placeHolder:'Review a server-proposed refactoring'});
            if(!chosen)return;
            response=choices[chosen.index].edit;
        }
        for(const [uri,base] of baselines)base.wireVersion=server.docs.get(uri)?.wireVersion??null;
        const rows=plan(response,baselines,ROOT);
        if(!rows.every(r=>sameBaseline(r.uri,r.base)))throw Error('project files changed');
        // Full before/after content is shown in a read-only virtual editor.
        // No raw source is logged and no server command is executed.
        previewText=rows.map(r=>'FILE: '+checkedPath(r.uri,ROOT).slice(ROOT.length+1)+'\n--- BEFORE ---\n'+r.base.text+'\n--- AFTER ---\n'+r.replacement).join('\n\n');
        if(Buffer.byteLength(previewText)>1024*1024)throw Error('preview too large');
        const preview=vscode.Uri.parse('nexus-lsp-review:/'+Date.now()+'/Workspace-edit.txt');
        await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(preview),{preview:true});
        const choice=await vscode.window.showWarningMessage('Apply reviewed text edits to '+rows.length+' file(s)? No automatic save.',{modal:true},'Apply reviewed edits');
        if(choice!=='Apply reviewed edits')return;
        if(stopping||!vscode.workspace.isTrusted||!rows.every(r=>sameBaseline(r.uri,r.base)))throw Error('workspace changed during review');
        const edit=new vscode.WorkspaceEdit();
        for(const r of rows){
            for(const e of r.rows){
                const a=e.range.start,b=e.range.end;
                edit.replace(vscode.Uri.parse(r.uri),new vscode.Range(a.line,a.character,b.line,b.character),e.text);
            }
        }
        if(!await vscode.workspace.applyEdit(edit))throw Error('editor rejected workspace edit');
        vscode.window.showInformationMessage('Reviewed workspace edit applied. Review and save changed buffers.');
    }catch(_){vscode.window.showWarningMessage('Project operation unavailable, stale, or outside reviewed text-only limits. No server command executed.');}
    finally{previewText='';projectBusy=false;}
}
async function approveOptional(){
    if(!vscode.workspace.isTrusted||stopping)return;
    const options=Object.entries(PROFILES).filter(([,p])=>p.optional).map(([key,p])=>({key,label:p.name,description:TOOL_ROOT+'/'+p.entry}));
    const selected=await vscode.window.showQuickPick(options,{placeHolder:'Enable a previously installed reviewed guest server'});
    if(!selected||sessions.has(selected.key))return;
    const version=await vscode.window.showInputBox({prompt:'Exact reviewed serverInfo.version (from the installed server)'});
    if(!version)return;
    const digest=await vscode.window.showInputBox({prompt:'SHA-256 of the reviewed entry file',validateInput:v=>/^[a-f0-9]{64}$/.test(v)?undefined:'Expected lowercase SHA-256'});
    if(!digest)return;
    const decision=await vscode.window.showWarningMessage('Run '+selected.label+' from '+selected.description+'? Review its entire installed toolchain and dependencies separately. The guest process can read guest data. This only verifies the entry file, not a sandbox or full package signature.',{modal:true},'Enable for this session');
    if(decision!=='Enable for this session')return;
    try{
        const server=new Session(selected.key,{approval:{version,sha256:digest},projectIndex});
        if(sessions.size>=4)throw Error('session limit');
        approvals.set(selected.key,{version,sha256:digest});sessions.set(selected.key,server);
        vscode.window.showInformationMessage('Reviewed entry verified; server will initialize on demand.');
    }catch(_){vscode.window.showWarningMessage('Reviewed server entry missing, changed or unsupported.');}
}
exports.activate=function(context){
    collection=vscode.languages.createDiagnosticCollection('nexus-reviewed-lsp');
    context.subscriptions.push(collection);
    context.subscriptions.push(vscode.workspace.registerTextDocumentContentProvider('nexus-lsp-review',{provideTextDocumentContent:()=>previewText}));
    const scheduled=new Map();
    // Debounced live diagnostics for normal files. Acceptance fixtures use
    // explicit saves, so tests still prove real editor persistence.
    function schedule(doc,delay){
        if(!supported(doc))return;
        const key=doc.uri.toString();
        if(scheduled.has(key))clearTimeout(scheduled.get(key));
        else if(scheduled.size>=24)return;
        scheduled.set(key,setTimeout(()=>{scheduled.delete(key);diagnostics(doc);},delay));
    }
    context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(doc=>{if(!fixture(doc))schedule(doc,800);}));
    context.subscriptions.push(vscode.workspace.onDidChangeTextDocument(e=>{
        collection.delete(e.document.uri);
        if(e.contentChanges.length&&!fixture(e.document))schedule(e.document,800);
    }));
    context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(doc=>{
        collection.delete(doc.uri);
        const key=doc.uri.toString();if(scheduled.has(key)){clearTimeout(scheduled.get(key));scheduled.delete(key);}
        const s=sessions.get(profileFor(doc.languageId));if(s)s.closeDocument(doc.uri.toString()).catch(()=>{});
    }));
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(doc=>{
        if(!supported(doc))return;
        const f=fixture(doc),text=doc.getText();
        if(f&&text===f.initial+f.suffix&&completed.has(f.filename)){
            marker(f.savedMarker,'saved');return;
        }
        schedule(doc,150);
    }));
    context.subscriptions.push({dispose(){for(const timer of scheduled.values())clearTimeout(timer);scheduled.clear();}});
    for(const language of Object.keys(LANGUAGES)){
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
    for(const language of Object.keys(LANGUAGES)){
        if(PROFILES[profileFor(language)].snapshot)continue;
        const selector={scheme:'file',language};
        context.subscriptions.push(vscode.languages.registerDefinitionProvider(selector,{
            async provideDefinition(doc,pos,token){
                try{const {result}=await projectRPC(doc,'textDocument/definition',{position:{line:pos.line,character:pos.character}},token);
                    return (Array.isArray(result)?result:[result]).slice(0,200).map(location).filter(Boolean);
                }catch(_){return [];}
            }
        }));
        context.subscriptions.push(vscode.languages.registerReferenceProvider(selector,{
            async provideReferences(doc,pos,ctx,token){
                try{const {result}=await projectRPC(doc,'textDocument/references',{position:{line:pos.line,character:pos.character},context:{includeDeclaration:!!ctx.includeDeclaration}},token);
                    return (Array.isArray(result)?result:[]).slice(0,500).map(location).filter(Boolean);
                }catch(_){return [];}
            }
        }));
    }
    context.subscriptions.push(vscode.languages.registerWorkspaceSymbolProvider({
        async provideWorkspaceSymbols(query,token){
            const doc=vscode.window.activeTextEditor?.document;
            if(!doc||typeof query!=='string'||query.length>128)return [];
            try{
                const {result}=await projectRPC(doc,'workspace/symbol',{query},token);
                return (Array.isArray(result)?result:[]).slice(0,300).flatMap(row=>{
                    const loc=location(row.location);
                    return loc&&typeof row.name==='string'&&row.name.length<=160&&Number.isInteger(row.kind)&&row.kind>=1&&row.kind<=26?
                        [new vscode.SymbolInformation(row.name,row.kind-1,typeof row.containerName==='string'?row.containerName.slice(0,160):'',loc)]:[];
                });
            }catch(_){return [];}
        }
    }));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.projectFixture',async()=>{
        if(!vscode.workspace.isTrusted||stopping||projectBusy||!Object.values(fixtures).every(f=>completed.has(f.filename)))return;
        projectBusy=true;
        try{
            // An explicit, synthetic test only. Create a new directory with
            // fixed filenames, never overwrite existing project files.
            const folder=path.join(ROOT,'nexus-project-acceptance');
            fs.mkdirSync(folder);
            const header=path.join(folder,'shared.hpp'),source=path.join(folder,'main.cpp');
            const h='inline int nexus_shared(int x) { return x + 1; }\n';
            const c='#include "shared.hpp"\nint nexus_entry() { return nexus_shared(1); }\n';
            fs.writeFileSync(header,h,{flag:'wx'});fs.writeFileSync(source,c,{flag:'wx'});
            const huri=vscode.Uri.file(header),curi=vscode.Uri.file(source);
            const hd=await vscode.workspace.openTextDocument(huri),cd=await vscode.workspace.openTextDocument(curi);
            // Keep both buffers open for the real clangd cross-file index.
            await vscode.window.showTextDocument(hd,{preview:false});
            await vscode.window.showTextDocument(cd,{preview:false});
            const pos={line:1,character:28};
            const def=await projectRPC(cd,'textDocument/definition',{position:pos});
            const defs=(Array.isArray(def.result)?def.result:[def.result]).map(location).filter(Boolean);
            if(!defs.some(l=>l.uri.toString()===huri.toString()))throw Error('cross-file definition missing');
            const refs=await projectRPC(cd,'textDocument/references',{position:pos,context:{includeDeclaration:true}});
            const found=new Set((Array.isArray(refs.result)?refs.result:[]).map(location).filter(Boolean).map(l=>l.uri.toString()));
            if(!found.has(huri.toString())||!found.has(curi.toString()))throw Error('cross-file references missing');
            const baselines=await captureBaselines();
            const renamed=await projectRPC(cd,'textDocument/rename',{position:pos,newName:'nexus_renamed'});
            for(const [uri,base] of baselines)base.wireVersion=renamed.server.docs.get(uri)?.wireVersion??null;
            const rows=plan(renamed.result,baselines,ROOT);
            const expected=new Map([[huri.toString(),h.replace('nexus_shared','nexus_renamed')],[curi.toString(),c.replace('nexus_shared','nexus_renamed')]]);
            if(rows.length!==2||!rows.every(r=>expected.get(r.uri)===r.replacement&&sameBaseline(r.uri,r.base)))throw Error('cross-file rename mismatch');
            // This command is explicit approval for the two newly created
            // synthetic files only, not user project files.
            const edit=new vscode.WorkspaceEdit();
            for(const r of rows)for(const e of r.rows){
                const a=e.range.start,b=e.range.end;
                edit.replace(vscode.Uri.parse(r.uri),new vscode.Range(a.line,a.character,b.line,b.character),e.text);
            }
            if(!await vscode.workspace.applyEdit(edit)||!await hd.save()||!await cd.save())throw Error('fixture edit failed');
            if(![...expected].every(([uri,text])=>boundedFile(uri)===text))throw Error('fixture save mismatch');
            projectAcceptance=true;
            marker('nexus-project-lsp.marker','definition-references-rename');
        }catch(_){vscode.window.showWarningMessage('Cross-file project acceptance failed');}
        finally{projectBusy=false;}
    }));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.approve',approveOptional));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.rename',()=>reviewProjectEdit(vscode.window.activeTextEditor,'rename')));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.refactor',()=>reviewProjectEdit(vscode.window.activeTextEditor,'refactor')));
    context.subscriptions.push(vscode.commands.registerCommand('nexus.lsp.index',async()=>{
        if(!vscode.workspace.isTrusted||projectBusy)return;
        const choice=await vscode.window.showWarningMessage('Enable clangd background project indexing? It uses guest CPU, RAM and disk. Only reviewed workspace/build database. Project configuration and clang-tidy remain disabled.',{modal:true},'Enable indexing');
        if(choice!=='Enable indexing')return;
        if(!await stopAll(false))return;
        sessions.clear();accepted.clear();completed.clear();projectIndex=true;stopping=false;
    }));
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
        if(!await stopAll(false)){vscode.window.showWarningMessage('A language server did not exit. Relaunch the guest before restarting.');return;}
        sessions.clear();stopping=false;
        accepted.clear();completed.clear();projectAcceptance=false;
    }));
};
exports.deactivate=()=>stopAll(false);
