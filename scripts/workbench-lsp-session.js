'use strict';
// Persistent, bounded stdio LSP client. No editor API or host dependencies.
const {spawn}=require('child_process');
const {randomBytes}=require('crypto');
const path=require('path');
const {pathToFileURL,fileURLToPath}=require('url');
const ROOT='/root/nexus-workbench';
const PROFILES=Object.freeze({
    python:Object.freeze({name:'pylsp',version:/^1\.10\.0$/,snapshot:true,command:'/usr/bin/python3',args:['-u','-m','pylsp']}),
    clangd:Object.freeze({name:'clangd',version:/\b18\.1\.3\b/,snapshot:false,command:'/usr/bin/clangd-18',args:['--background-index=false','--clang-tidy=false','--enable-config=false','--pch-storage=memory','-j=1','--offset-encoding=utf-16','--header-insertion=never','--header-insertion-decorators=false','--log=error']})
});
function profileFor(language){return language==='python'?'python':language==='c'||language==='cpp'?'clangd':null;}
function boundedSnapshot(doc,root){
    if(!doc||typeof doc.uri!=='string'||typeof doc.text!=='string'||!Number.isSafeInteger(doc.version)||doc.version<1||
        Buffer.byteLength(doc.text,'utf8')>131072||!profileFor(doc.language))throw Error('invalid snapshot');
    const filename=fileURLToPath(doc.uri);
    if(!path.resolve(filename).startsWith(path.resolve(root)+'/')||!path.isAbsolute(filename))throw Error('outside workspace');
    return filename;
}
class Session{
    constructor(key,{root=ROOT,timeout=120000,spawnProcess=spawn}={}){
        if(!Object.hasOwn(PROFILES,key))throw Error('unknown server');
        this.profile=PROFILES[key];this.key=key;this.root=root;this.timeout=timeout;this.spawnProcess=spawnProcess;
        this.nonce=randomBytes(8).toString('hex');this.sequence=0;this.id=0;
        this.pending=new Map();this.docs=new Map();this.wireDocs=new Map();
        this.buffer=Buffer.alloc(0);this.failure=null;this.child=null;this.starting=null;
        this.queue=Promise.resolve();this.queued=0;this.closing=false;this.closePromise=null;
        this.stats={launches:0,initializes:0,opens:0,changes:0,snapshotRefreshes:0,diagnostics:0,completions:0,closes:0,shutdownClean:false,peakDocuments:0};
    }
    enqueue(work){
        if(this.closing||this.failure||this.queued>=8)return Promise.reject(Error('session unavailable'));
        this.queued++;
        const job=this.queue.then(()=>{if(this.failure)throw this.failure;return work();});
        this.queue=job.catch(()=>{}).finally(()=>{this.queued--;});
        return job;
    }
    fail(error){
        if(this.failure)return;
        this.failure=error;
        for(const p of this.pending.values()){clearTimeout(p.timer);p.cancellation?.dispose();p.reject(error);}
        this.pending.clear();
        for(const d of this.docs.values())this.rejectWaiter(d,error);
        if(this.child&&this.child.exitCode===null&&this.child.signalCode===null)this.child.kill('SIGKILL');
    }
    rejectWaiter(d,error){
        if(d.waiter){const w=d.waiter;d.waiter=null;clearTimeout(w.timer);w.reject(error);}
    }
    send(msg){
        if(this.failure)throw this.failure;
        const body=Buffer.from(JSON.stringify({jsonrpc:'2.0',...msg}),'utf8');
        if(body.length>524288||this.child.stdin.writableLength>1048576)throw Error('LSP output limit');
        this.child.stdin.write(Buffer.concat([Buffer.from('Content-Length: '+body.length+'\r\n\r\n'),body]));
    }
    notify(method,params){this.send({method,params});}
    call(method,params,token){
        if(this.failure)return Promise.reject(this.failure);
        if(token?.isCancellationRequested)return Promise.reject(Error('cancelled'));
        if(this.pending.size>=8)return Promise.reject(Error('pending limit'));
        const id=++this.id;
        return new Promise((resolve,reject)=>{
            const timer=setTimeout(()=>this.fail(Error('LSP request timeout')),this.timeout);
            const p={resolve,reject,timer,cancellation:null,cancelled:false};
            this.pending.set(id,p);
            try{
                this.send({id,method,params});
                if(token)p.cancellation=token.onCancellationRequested(()=>{
                    // Retain id until response; a late response cannot satisfy another request.
                    if(this.pending.has(id)){p.cancelled=true;try{this.notify('$/cancelRequest',{id});}catch(e){this.fail(e);}}
                });
            }catch(e){this.fail(e);}
        });
    }
    receive(chunk){
        try{
            this.buffer=Buffer.concat([this.buffer,chunk]);
            if(this.buffer.length>1048576)throw Error('LSP buffer limit');
            while(true){
                const end=this.buffer.indexOf('\r\n\r\n');
                if(end<0){if(this.buffer.length>4096)throw Error('header limit');return;}
                if(end>4096)throw Error('header limit');
                const headers=this.buffer.subarray(0,end).toString('ascii').split('\r\n');
                const lengths=headers.filter(h=>/^Content-Length: [0-9]+$/i.test(h));
                if(lengths.length!==1||headers.some(h=>!/^Content-(Length: [0-9]+|Type: [^\r\n]+)$/i.test(h)))throw Error('LSP header');
                const count=Number(lengths[0].split(':')[1]),start=end+4;
                if(!Number.isSafeInteger(count)||count<1||count>524288)throw Error('LSP frame limit');
                if(this.buffer.length<start+count)return;
                const msg=JSON.parse(this.buffer.subarray(start,start+count).toString('utf8'));
                this.buffer=this.buffer.subarray(start+count);
                this.dispatch(msg);
            }
        }catch(e){this.fail(e);}
    }
    dispatch(msg){
        if(!msg||msg.jsonrpc!=='2.0'||Array.isArray(msg))throw Error('invalid RPC');
        if(typeof msg.method==='string'){
            if(msg.id!==undefined){this.send({id:msg.id,error:{code:-32601,message:'Unsupported server request'}});return;}
            if(msg.method!=='textDocument/publishDiagnostics')return;
            const p=msg.params,d=this.wireDocs.get(p?.uri);
            if(!d||!Array.isArray(p.diagnostics))return;
            // Python's pinned server omits version: never reuse a wire URI.
            // Clangd uses actual URI; version is mandatory, no stale projection.
            if(this.profile.snapshot?(p.version!==undefined&&p.version!==d.wireVersion):p.version!==d.wireVersion)return;
            if(p.diagnostics.length>2000)throw Error('diagnostic limit');
            d.diagnostics=p.diagnostics;
            this.stats.diagnostics++;
            if(d.waiter){
                const w=d.waiter;d.waiter=null;clearTimeout(w.timer);w.resolve(d.diagnostics);
            }
            return;
        }
        const p=this.pending.get(msg.id);
        if(!p)throw Error('unknown reply');
        this.pending.delete(msg.id);clearTimeout(p.timer);p.cancellation?.dispose();
        if(p.cancelled||msg.error)p.reject(Error(p.cancelled?'cancelled':'server request rejected'));
        else if(Object.hasOwn(msg,'result'))p.resolve(msg.result);
        else {p.reject(Error('missing result'));throw Error('missing result');}
    }
    async start(){
        if(this.failure)throw this.failure;
        if(this.starting)return this.starting;
        this.starting=(async()=>{
            const p=this.profile;
            this.child=this.spawnProcess(p.command,p.args,{cwd:this.root,stdio:['pipe','pipe','ignore'],
                env:{PATH:'/usr/bin:/bin',HOME:'/root',LANG:'C.UTF-8',PYTHONFAULTHANDLER:'1'}});
            this.stats.launches++;
            this.exited=new Promise(resolve=>this.child.once('close',(code,signal)=>resolve({code,signal})));
            this.child.on('error',e=>this.fail(e));
            this.child.stdin.on('error',e=>this.fail(e));
            this.child.stdout.on('data',b=>this.receive(b));
            this.exited.then(()=>{if(!this.closing)this.fail(Error('LSP process exited'));});
            const result=await this.call('initialize',{processId:null,rootUri:pathToFileURL(this.root).href,workspaceFolders:[{uri:pathToFileURL(this.root).href,name:'Nexus workspace'}],
                capabilities:{general:{positionEncodings:['utf-16']},offsetEncoding:['utf-16'],textDocument:{publishDiagnostics:{versionSupport:true},completion:{completionItem:{snippetSupport:false}},synchronization:{dynamicRegistration:false}}}});
            const caps=result?.capabilities,sync=caps?.textDocumentSync;
            const kind=typeof sync==='number'?sync:sync?.openClose===true?sync.change:undefined;
            if(result?.serverInfo?.name!==p.name||!p.version.test(result.serverInfo.version)||!caps?.completionProvider||
                ![1,2].includes(kind)||(caps.positionEncoding&&caps.positionEncoding!=='utf-16')||(result.offsetEncoding&&result.offsetEncoding!=='utf-16'))throw Error('unexpected server identity or capabilities');
            this.stats.initializes++;this.notify('initialized',{});
        })().catch(e=>{this.fail(e);throw e;});
        return this.starting;
    }
    async sync(doc){
        const filename=boundedSnapshot(doc,this.root);
        if(profileFor(doc.language)!==this.key)throw Error('wrong server');
        await this.start();
        let d=this.docs.get(doc.uri);
        if(d&&d.version===doc.version&&d.text===doc.text)return d;
        if(d&&doc.version<=d.version)throw Error('stale editor snapshot');
        if(!d&&this.docs.size>=12)throw Error('document limit');
        if(d&&this.profile.snapshot){
            this.drop(d);d=null;this.stats.snapshotRefreshes++;
        }
        if(!d){
            const ext=path.extname(filename);
            if(this.profile.snapshot&&!ext)throw Error('missing extension');
            const wire=this.profile.snapshot?pathToFileURL(path.join(path.dirname(filename),'.nexus-lsp-'+this.nonce+'-'+(++this.sequence)+ext)).href:doc.uri;
            d={uri:doc.uri,wire,wireVersion:this.profile.snapshot?1:++this.sequence,version:doc.version,text:doc.text,language:doc.language,diagnostics:null,waiter:null};
            this.docs.set(doc.uri,d);this.wireDocs.set(wire,d);
            this.notify('textDocument/didOpen',{textDocument:{uri:wire,languageId:doc.language,version:d.wireVersion,text:doc.text}});
            this.stats.opens++;this.stats.peakDocuments=Math.max(this.stats.peakDocuments,this.docs.size);
        }else{
            this.rejectWaiter(d,Error('document changed'));
            d.version=doc.version;d.text=doc.text;d.wireVersion=++this.sequence;d.diagnostics=null;
            // Full content change is permitted even for incremental sync.
            this.notify('textDocument/didChange',{textDocument:{uri:d.wire,version:d.wireVersion},contentChanges:[{text:doc.text}]});
            this.stats.changes++;
        }
        return d;
    }
    drop(d){
        this.rejectWaiter(d,Error('document closed'));
        this.docs.delete(d.uri);this.wireDocs.delete(d.wire);
        this.notify('textDocument/didClose',{textDocument:{uri:d.wire}});
        this.stats.closes++;
    }
    diagnostic(doc){
        return this.enqueue(async()=>{
            const d=await this.sync(doc);
            if(d.diagnostics!==null)return d.diagnostics;
            return new Promise((resolve,reject)=>{
                const timer=setTimeout(()=>this.fail(Error('LSP diagnostic timeout')),this.timeout);
                d.waiter={resolve,reject,timer};
            });
        });
    }
    complete(doc,position,token){
        return this.enqueue(async()=>{
            if(token?.isCancellationRequested)throw Error('cancelled');
            const d=await this.sync(doc);
            const result=await this.call('textDocument/completion',{textDocument:{uri:d.wire},position,context:{triggerKind:1}},token);
            this.stats.completions++;
            const items=Array.isArray(result)?result:result?.items;
            if(!Array.isArray(items))throw Error('invalid completion result');
            return items.slice(0,1000);
        });
    }
    closeDocument(uri){return this.enqueue(()=>{const d=this.docs.get(uri);if(d)this.drop(d);});}
    stop(){
        if(this.closePromise)return this.closePromise;
        this.closing=true;
        this.closePromise=(async()=>{
            if(!this.child)return {...this.stats};
            let timer;
            try{
                await Promise.race([this.queue,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('drain timeout')),10000);})]);
                clearTimeout(timer);
                if(this.failure)throw this.failure;
                await this.starting;
                for(const d of [...this.docs.values()])this.drop(d);
                if(await Promise.race([this.call('shutdown',null),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('shutdown timeout')),10000);})])!==null)throw Error('shutdown response');
                clearTimeout(timer);
                this.notify('exit');this.child.stdin.end();
                const outcome=await Promise.race([this.exited,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('exit timeout')),10000);})]);
                if(outcome.code!==0||outcome.signal!==null)throw Error('unclean exit');
                this.stats.shutdownClean=true;
            }catch(e){
                this.fail(e);clearTimeout(timer);
                await Promise.race([this.exited,new Promise(resolve=>{timer=setTimeout(resolve,2000);})]);
            }
            finally{clearTimeout(timer);}
            return {...this.stats};
        })();
        return this.closePromise;
    }
}
module.exports={Session,profileFor,PROFILES,ROOT};
