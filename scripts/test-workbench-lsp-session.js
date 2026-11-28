'use strict';
// Synthetic transport regression only, separate from actual Workbench gate.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const {EventEmitter}=require('node:events');
const {PassThrough,Writable}=require('node:stream');
const {Session,ROOT}=require('./workbench-lsp-session');
function fake(key){
    const messages=[],child=new EventEmitter();
    child.stdout=new PassThrough();child.exitCode=null;child.signalCode=null;
    function emit(msg,fragment=false){
        const data=Buffer.from(JSON.stringify({jsonrpc:'2.0',...msg}));
        const frame=Buffer.concat([Buffer.from('Content-Length: '+data.length+'\r\n\r\n'),data]);
        if(fragment){child.stdout.write(frame.subarray(0,12));child.stdout.write(frame.subarray(12));}
        else child.stdout.write(frame);
    }
    child.kill=signal=>{if(child.exitCode===null&&child.signalCode===null){child.signalCode=signal;queueMicrotask(()=>child.emit('close',null,signal));}};
    child.stdin=new Writable({write(chunk,_enc,done){
        const b=Buffer.from(chunk),end=b.indexOf('\r\n\r\n'),msg=JSON.parse(b.subarray(end+4));
        messages.push(msg);
        if(msg.method==='initialize')queueMicrotask(()=>emit({id:msg.id,result:{serverInfo:{name:key==='python'?'pylsp':'clangd',version:key==='python'?'1.10.0':'18.1.3'},capabilities:{textDocumentSync:{openClose:true,change:2},completionProvider:{},positionEncoding:'utf-16'}}},true));
        if(msg.method==='textDocument/completion')queueMicrotask(()=>emit({id:msg.id,result:[{label:'value',insertText:'value'}]}));
        if(msg.method==='shutdown')queueMicrotask(()=>emit({id:msg.id,result:null}));
        if(msg.method==='exit'){child.exitCode=0;queueMicrotask(()=>child.emit('close',0,null));}
        done();
    }});
    return {messages,child,emit,spawnProcess:()=>child};
}
const tick=()=>new Promise(resolve=>setImmediate(resolve));
async function until(fn){for(let i=0;i<100;i++){const value=fn();if(value)return value;await tick();}throw Error('fixture did not advance');}
function doc(name,version,text,language){return {uri:'file://'+ROOT+'/'+name,version,text,language};}
function diag(wire,version,message){return {method:'textDocument/publishDiagnostics',params:{uri:wire,...(version===undefined?{}:{version}),diagnostics:[{message}]}};}
test('Python process reuse, retired URI ignored, multi-document close and shutdown',async()=>{
    const f=fake('python'),s=new Session('python',{spawnProcess:f.spawnProcess,timeout:1000});
    const a=doc('a.py',1,'x','python'),b=doc('b.py',1,'y','python');
    const first=s.diagnostic(a);
    const m=await until(()=>f.messages.find(x=>x.method==='textDocument/didOpen'));
    f.emit(diag(m.params.textDocument.uri,undefined,'first'),true);
    assert.deepEqual(await first,[{message:'first'}]);
    const second=s.diagnostic({...a,version:2,text:'new'});
    const next=await until(()=>f.messages.filter(x=>x.method==='textDocument/didOpen')[1]);
    assert.notEqual(m.params.textDocument.uri,next.params.textDocument.uri);
    f.emit(diag(m.params.textDocument.uri,undefined,'stale'));
    f.emit(diag(next.params.textDocument.uri,2,'wrong version'));
    f.emit(diag(next.params.textDocument.uri,undefined,'fresh'));
    assert.deepEqual(await second,[{message:'fresh'}]);
    assert.equal((await s.complete(b,{line:0,character:1}))[0].label,'value');
    await s.closeDocument(a.uri);
    const r=await s.stop();
    assert.equal(r.launches,1);assert.equal(r.initializes,1);assert.equal(r.peakDocuments,2);
    assert.equal(r.snapshotRefreshes,1);assert.equal(r.opens,r.closes);assert.equal(r.shutdownClean,true);
});
test('clangd same URI versioned changes, stale diagnostics rejected after close/reopen',async()=>{
    const f=fake('clangd'),s=new Session('clangd',{spawnProcess:f.spawnProcess,timeout:1000});
    const a=doc('a.cpp',1,'x','cpp');
    const first=s.diagnostic(a);
    const m=await until(()=>f.messages.find(x=>x.method==='textDocument/didOpen'));
    const version=m.params.textDocument.version;
    f.emit(diag(a.uri,undefined,'unversioned'));
    f.emit(diag(a.uri,version,'first'));
    assert.deepEqual(await first,[{message:'first'}]);
    const second=s.diagnostic({...a,version:2,text:'y'});
    const change=await until(()=>f.messages.find(x=>x.method==='textDocument/didChange'));
    assert.equal(change.params.textDocument.uri,a.uri);
    f.emit(diag(a.uri,version,'stale'));f.emit(diag(a.uri,change.params.textDocument.version,'fresh'));
    assert.deepEqual(await second,[{message:'fresh'}]);
    await s.closeDocument(a.uri);
    const reopen=s.diagnostic(a);
    const next=await until(()=>f.messages.filter(x=>x.method==='textDocument/didOpen')[1]);
    assert.ok(next.params.textDocument.version>change.params.textDocument.version);
    f.emit(diag(a.uri,version,'old'));f.emit(diag(a.uri,next.params.textDocument.version,'reopened'));
    assert.deepEqual(await reopen,[{message:'reopened'}]);
    await s.complete(doc('b.c',1,'x','c'),{line:0,character:1});
    const r=await s.stop();
    assert.equal(r.launches,1);assert.equal(r.changes,1);assert.equal(r.peakDocuments,2);assert.equal(r.shutdownClean,true);
});
test('outside workspace and wrong language rejected, malformed frame kills session',async()=>{
    const f=fake('python'),s=new Session('python',{spawnProcess:f.spawnProcess,timeout:1000});
    await assert.rejects(s.complete(doc('../outside.py',1,'x','python'),{line:0,character:0}));
    await assert.rejects(s.complete(doc('wrong.cpp',1,'x','cpp'),{line:0,character:0}));
    const waiting=s.diagnostic(doc('a.py',1,'x','python'));
    await until(()=>f.messages.find(x=>x.method==='textDocument/didOpen'));
    f.child.stdout.write('Content-Length: 999999999\r\n\r\n');
    await assert.rejects(waiting);
    assert.ok(s.failure);assert.equal(f.child.signalCode,'SIGKILL');
    assert.equal((await s.stop()).shutdownClean,false);
});

test('RPC rejection preserves method and code without server content',async()=>{
    const f=fake('clangd'),s=new Session('clangd',{spawnProcess:f.spawnProcess,timeout:1000});
    try{
        await s.start();
        const pending=s.call('textDocument/definition',{});
        const msg=await until(()=>f.messages.find(x=>x.method==='textDocument/definition'));
        f.emit({id:msg.id,error:{code:-32801,message:'private source content',data:{secret:'private path'}}});
        await assert.rejects(pending,e=>e.rpcCode===-32801&&e.rpcMethod==='textDocument/definition'&&!e.message.includes('private'));
        assert.equal(s.failure,null);
    }finally{assert.equal((await s.stop()).shutdownClean,true);}
});
test('index probe retries cancellation but never semantic errors or missing evidence',async()=>{
    const {indexedRequest}=require('./test-real-native-language-lsp');
    let attempts=0;
    const transient=Object.assign(Error('cancelled'),{rpcCode:-32801,rpcMethod:'textDocument/definition'});
    const options={timeout:1000,maxAttempts:3,pause:1};
    assert.equal(await indexedRequest(async()=>{
        attempts++;
        if(attempts===1)throw transient;
        return attempts===2?null:'verified';
    },r=>r==='verified',options),'verified');
    assert.equal(attempts,3);
    attempts=0;
    await assert.rejects(indexedRequest(async()=>{
        attempts++;throw Object.assign(Error('semantic rejection'),{rpcCode:-32603});
    },undefined,options),/semantic rejection/);
    assert.equal(attempts,1);
    attempts=0;
    await assert.rejects(indexedRequest(async()=>{attempts++;return null;},Boolean,options),/required evidence/);
    assert.equal(attempts,3);
});
