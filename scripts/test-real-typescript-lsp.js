'use strict';
// Real npm-pinned TypeScript server, no simulated protocol peer.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {createHash}=require('node:crypto');
const {pathToFileURL}=require('node:url');
const packaged=fs.existsSync(path.join(__dirname,'lsp-session.js'));
const {Session}=require(packaged?'./lsp-session':'./workbench-lsp-session');
const {plan}=require(packaged?'./lsp-edits':'./workbench-lsp-edits');
async function run(inputRoot){
    const toolRoot=fs.realpathSync(inputRoot);
    const entry=path.join(toolRoot,'typescript/node_modules/typescript-language-server/lib/cli.mjs');
    const sha256=createHash('sha256').update(fs.readFileSync(entry)).digest('hex');
    const root=fs.mkdtempSync(path.join(os.tmpdir(),'nexus-real-ts-'));
    const home=path.join(root,'home');fs.mkdirSync(home);
    const s=new Session('typescript',{root,toolRoot,nodeExecutable:process.execPath,home,approval:{version:'4.3.4',sha256},timeout:30000});
    try{
        fs.writeFileSync(path.join(root,'tsconfig.json'),JSON.stringify({compilerOptions:{allowJs:true,checkJs:true,noEmit:true,module:'commonjs'},include:['*.ts','*.js']}));
        for(const language of ['typescript','javascript']){
            const ext=language==='typescript'?'ts':'js',name=language==='typescript'?'tsValue':'jsValue',replacement=name+'Renamed';
            const a={uri:pathToFileURL(path.join(root,'lib-'+ext+'.'+ext)).href,language,version:1,text:'export const '+name+' = 42;\n'};
            const b={uri:pathToFileURL(path.join(root,'main-'+ext+'.'+ext)).href,language,version:1,text:'import { '+name+' } from "./lib-'+ext+'";\nconsole.log('+name+');\n'};
            fs.writeFileSync(new URL(a.uri),a.text);fs.writeFileSync(new URL(b.uri),b.text);
            const docs=[a,b],pos={line:1,character:13};
            const def=await s.projectRequest(b,docs,'textDocument/definition',{position:pos});
            assert.ok((Array.isArray(def)?def:[def]).some(r=>(r.uri||r.targetUri)===a.uri),language+' cross-file definition');
            const refs=await s.projectRequest(b,docs,'textDocument/references',{position:pos,context:{includeDeclaration:true}});
            assert.ok(refs.some(r=>r.uri===a.uri)&&refs.some(r=>r.uri===b.uri),language+' references');
            const edit=await s.projectRequest(a,docs,'textDocument/rename',{position:{line:0,character:14},newName:replacement});
            const baseline=new Map(docs.map(d=>[d.uri,{text:d.text,wireVersion:s.docs.get(d.uri).wireVersion}]));
            const rows=plan(edit,baseline,root);
            assert.equal(rows.length,2,language+' rename affected files');
            for(const r of rows){
                assert.equal(r.replacement,baseline.get(r.uri).text.replaceAll(name,replacement));
            }
            const changed={...b,version:2,text:b.text+'Math.ma'};
            const completions=await s.complete(changed,{line:2,character:7});
            assert.ok(completions.some(i=>i.label==='max'),language+' real completion');
            await s.closeDocument(a.uri);await s.closeDocument(b.uri);
            console.log(language+': real definition, references, two-file rename and completion PASS');
        }
        const report=await s.stop();
        assert.equal(report.launches,1);assert.equal(report.initializes,1);assert.equal(report.changes,2);
        assert.equal(report.shutdownClean,true);assert.equal(report.opens,report.closes);
        return {ok:true,scope:'real pinned TypeScript LSP, not physical-device certification',server:'4.3.4',typescript:'5.7.3',entrySHA256:sha256,stats:report};
    }finally{await s.stop();fs.rmSync(root,{recursive:true,force:true});}
}
module.exports={run};
if(require.main===module)run(process.argv[2]).then(report=>console.log(JSON.stringify(report))).catch(e=>{console.error(e.stack);process.exitCode=1;});
