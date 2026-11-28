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
        // Indexing is asynchronous in real servers. Retry only successful
        // empty responses (or capability not yet ready), never fabricate proof.
        const deadline=Date.now()+120000;
        let definitions=[];
        while(Date.now()<deadline){
            const result=await s.projectRequest(b,docs,'textDocument/definition',{position});
            definitions=(Array.isArray(result)?result:[result]).filter(Boolean);
            if(definitions.some(r=>(r.uri||r.targetUri)===a.uri))break;
            await new Promise(resolve=>setTimeout(resolve,500));
        }
        assert.ok(definitions.some(r=>(r.uri||r.targetUri)===a.uri),key+' cross-file definition');
        const refs=await s.projectRequest(a,docs,'textDocument/references',{position:declPosition,context:{includeDeclaration:true}});
        assert.ok(Array.isArray(refs)&&refs.some(r=>r.uri===a.uri)&&refs.some(r=>r.uri===b.uri),key+' cross-file references');
        const edit=await s.projectRequest(a,docs,'textDocument/rename',{position:declPosition,newName});
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
module.exports={run};
