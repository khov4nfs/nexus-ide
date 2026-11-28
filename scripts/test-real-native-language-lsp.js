'use strict';
// Real native-toolchain project protocol probe, no synthetic server.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {createHash}=require('node:crypto');
const {pathToFileURL}=require('node:url');
const {Session}=require('./workbench-lsp-session');
const {plan}=require('./workbench-lsp-edits');
// Index invalidation can cancel an in-flight request even after initialize.
// The probe has immutable fixture snapshots and no user cancellation token.
// Retry only standard transient LSP errors; never retry a semantic rejection,
// transport failure, invalid edit, or other JSON-RPC error.
async function indexedRequest(request,accept=()=>true,{timeout=120000,maxAttempts=120,pause=500}={}){
    const deadline=Date.now()+timeout;
    for(let attempt=0;attempt<maxAttempts&&Date.now()<deadline;attempt++){
        try{
            const result=await request();
            if(accept(result))return result;
        }catch(e){
            if(![-32800,-32801,-32802].includes(e.rpcCode))throw e;
            console.error('Transient index request cancellation: '+e.rpcMethod+' ('+e.rpcCode+')');
        }
        if(attempt+1<maxAttempts&&Date.now()<deadline)await new Promise(resolve=>setTimeout(resolve,pause));
    }
    throw Error('Index request did not produce required evidence before deadline');
}
async function run(key,toolRoot,version){
    if(!['go','rust'].includes(key))throw Error('unsupported test profile');
    toolRoot=fs.realpathSync(toolRoot);
    const entry=path.join(toolRoot,'bin',key==='go'?'gopls':'rust-analyzer');
    const sha256=createHash('sha256').update(fs.readFileSync(entry)).digest('hex');
    const root=fs.mkdtempSync(path.join(os.tmpdir(),'nexus-project-lsp-'));
    const home=path.join(root,'home');fs.mkdirSync(home);
    const s=new Session(key,{root,toolRoot,home,approval:{version,sha256},timeout:90000});
    try{
        let docs,position,declPosition,newName,oldName;
        if(key==='go'){
            fs.writeFileSync(path.join(root,'go.mod'),'module example.invalid/nexus\n\ngo 1.23.4\n');
            docs=[
                ['shared.go','package nexus\n\nfunc SharedValue() int { return 42 }\n'],
                ['main.go','package nexus\n\nfunc Run() int { return SharedValue() }\n']
            ];
            position={line:2,character:26};declPosition={line:2,character:7};oldName='SharedValue';newName='RenamedValue';
        }else{
            fs.writeFileSync(path.join(root,'Cargo.toml'),'[package]\nname = "nexus-lsp-fixture"\nversion = "0.1.0"\nedition = "2021"\n[dependencies]\n');
            fs.mkdirSync(path.join(root,'src'));
            docs=[
                ['src/shared.rs','pub fn shared_value() -> i32 { 42 }\n'],
                ['src/lib.rs','mod shared;\npub fn run() -> i32 { shared::shared_value() }\n']
            ];
            position={line:1,character:34};declPosition={line:0,character:10};oldName='shared_value';newName='renamed_value';
        }
        docs=docs.map(([file,text])=>{
            const absolute=path.join(root,file);fs.writeFileSync(absolute,text);
            return {uri:pathToFileURL(absolute).href,language:key,version:1,text};
        });
        const [a,b]=docs;
        const hasDefinition=result=>(Array.isArray(result)?result:[result]).filter(Boolean).some(r=>(r.uri||r.targetUri)===a.uri);
        const hasReferences=result=>Array.isArray(result)&&result.some(r=>r.uri===a.uri)&&result.some(r=>r.uri===b.uri);
        const definitions=await indexedRequest(()=>s.projectRequest(b,docs,'textDocument/definition',{position}),hasDefinition);
        assert.ok(hasDefinition(definitions),key+' cross-file definition');
        const refs=await indexedRequest(()=>s.projectRequest(a,docs,'textDocument/references',{position:declPosition,context:{includeDeclaration:true}}),hasReferences);
        assert.ok(hasReferences(refs),key+' cross-file references');
        const edit=await indexedRequest(()=>s.projectRequest(a,docs,'textDocument/rename',{position:declPosition,newName}));
        const baseline=new Map(docs.map(d=>[d.uri,{text:d.text,wireVersion:s.docs.get(d.uri).wireVersion}]));
        const rows=plan(edit,baseline,root);
        assert.equal(rows.length,2,key+' rename file count');
        for(const row of rows)assert.equal(row.replacement,baseline.get(row.uri).text.replaceAll(oldName,newName));
        await s.closeDocument(a.uri);await s.closeDocument(b.uri);
        const report=await s.stop();
        assert.equal(report.launches,1);assert.equal(report.initializes,1);
        assert.equal(report.opens,2);assert.equal(report.closes,2);assert.equal(report.shutdownClean,true);
        return {ok:true,scope:'host real toolchain protocol, not iPhone',key,version,entrySHA256:sha256,definitionVerified:true,referencesVerified:true,renameVerified:true,stats:report};
    }finally{await s.stop();fs.rmSync(root,{recursive:true,force:true});}
}
if(require.main===module)run(process.argv[2],process.argv[3],process.argv[4]).then(r=>console.log(JSON.stringify(r))).catch(e=>{console.error(e.stack);process.exitCode=1;});
module.exports={run,indexedRequest};
