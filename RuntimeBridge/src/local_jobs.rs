//! Separate local jobs, never run concurrently with ACP or another boot.
use crate::{process_probe, ATTEMPTED};
use ish_embed_host::IshInstance;
use serde_json::{json, Value};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{collections::{HashMap,VecDeque}, ffi::{c_char,CStr,CString}, path::Path,
    sync::{Mutex,atomic::Ordering}};

pub(crate) enum Control { Input(Vec<u8>), Resize(u16,u16), Stop }
const AGENT_LIMIT:usize=10;
#[derive(Default)]
struct AgentMailbox {waiting:bool, pending:Option<String>, finish:bool, turns:Vec<Value>, handle:Option<String>}
#[derive(Clone)]
pub(crate) struct NativeSession {pub retain:bool,pub resume:Option<String>,before:Option<String>,after:Option<String>}
pub(crate) enum AgentNext {Prompt(String),Finish,Stop}
fn valid_agent_text(s:&str)->bool {valid_command(s)&&s.chars().all(|c|!c.is_control()||c=='\n'||c=='\t')}
pub(crate) fn valid_handle(s:&str)->bool {!s.is_empty()&&s.len()<=128&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_".contains(&b))}
pub(crate) fn valid_uuid(s:&str)->bool {
    s.len()==36&&s.bytes().enumerate().all(|(i,b)|if matches!(i,8|13|18|23){b==b'-'}else{b.is_ascii_hexdigit()})
}
pub(crate) fn agent_complete(prompt:&str,response:&str,handle:Option<&str>)->bool{
    let Ok(mut s)=STATE.lock()else{return false;};
    if s.stop||!s.running{return false;}
    let Some(a)=s.agent.as_mut()else{return false;};
    if a.turns.len()>=AGENT_LIMIT{return false;}
    a.turns.push(json!({"prompt":prompt,"response":response}));
    a.handle=handle.map(str::to_owned);
    a.waiting=a.turns.len()<AGENT_LIMIT;
    true
}
pub(crate) async fn agent_next()->AgentNext{
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(180);
    loop{
        {
            let Ok(mut s)=STATE.lock()else{return AgentNext::Stop;};
            if s.stop||!s.running{return AgentNext::Stop;}
            let Some(a)=s.agent.as_mut()else{return AgentNext::Stop;};
            if let Some(p)=a.pending.take(){a.waiting=false;return AgentNext::Prompt(p);}
            if a.finish||a.turns.len()>=AGENT_LIMIT||std::time::Instant::now()>=deadline{a.waiting=false;return AgentNext::Finish;}
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
fn agent_send(ticket:u32,text:Option<String>)->Value{
    let Ok(mut s)=STATE.lock()else{return fail("Agent status unavailable");};
    if !s.running||s.stop{return fail("No active agent session");}
    let Some(a)=s.agent.as_mut()else{return fail("No native agent session");};
    if !a.waiting||a.pending.is_some()||a.finish||a.turns.len()!=ticket as usize{return fail("Agent is not waiting for this turn");}
    if let Some(text)=text{
        if !valid_agent_text(&text){return fail("Invalid message");}
        a.pending=Some(text);
    }else{a.finish=true;}
    a.waiting=false;
    json!({"ok":true})
}
#[derive(Default)]
struct LiveLsp {version:u32,waiting:bool,pending:Option<String>,finish:bool,diagnostics:Vec<Value>}
static LIVE_LSP:Mutex<Option<LiveLsp>>=Mutex::new(None);
pub(crate) enum LspNext {Change(String),Finish,Stop}
pub(crate) fn lsp_publish(version:u32,diagnostics:Vec<Value>)->bool{
    if stop_requested(){return false;}
    let Ok(mut slot)=LIVE_LSP.lock()else{return false;};
    let Some(s)=slot.as_mut()else{return false;};
    s.version=version;s.diagnostics=diagnostics;s.waiting=version<10;true
}
pub(crate) async fn lsp_wait()->LspNext{
    let until=std::time::Instant::now()+std::time::Duration::from_secs(180);
    loop{
        if stop_requested(){return LspNext::Stop;}
        {
            let Ok(mut slot)=LIVE_LSP.lock()else{return LspNext::Stop;};
            let Some(s)=slot.as_mut()else{return LspNext::Stop;};
            if let Some(text)=s.pending.take(){s.waiting=false;return LspNext::Change(text);}
            if s.finish||s.version>=10||std::time::Instant::now()>=until{s.waiting=false;return LspNext::Finish;}
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
fn lsp_control(version:u32,text:Option<String>)->Value{
    if stop_requested(){return fail("LSP stopped");}
    let Ok(mut slot)=LIVE_LSP.lock()else{return fail("LSP unavailable");};
    let Some(s)=slot.as_mut()else{return fail("No live LSP");};
    if s.version!=version||!s.waiting||s.finish||s.pending.is_some(){return fail("Stale or busy LSP version");}
    if let Some(text)=text{if text.len()>8192||text.contains('\0'){return fail("Invalid snapshot");}s.pending=Some(text);s.diagnostics.clear();}else{s.finish=true;}
    s.waiting=false;json!({"ok":true})
}
fn lsp_status()->Value{
    LIVE_LSP.lock().ok().and_then(|slot|slot.as_ref().map(|s|json!({"active":true,"version":s.version,"waiting":s.waiting,"diagnostics":s.diagnostics}))).unwrap_or_else(||json!({"active":false}))
}
struct State {
    running: bool, ready: bool, mode: u8, output: String, raw: Vec<u8>, total: u64,
    controls: VecDeque<Control>, stop: bool, agent:Option<AgentMailbox>,
}
static STATE: Mutex<State> = Mutex::new(State {
    running:false, ready:false, mode:0, output:String::new(), raw:Vec::new(), total:0, controls:VecDeque::new(), stop:false, agent:None,
});
const WORKBENCH: &str = "umask 077; mkdir -p /root/nexus-workbench /root/nexus-vscode-data /root/nexus-vscode-extensions && exec /opt/nexus/workbench/node --no-opt --no-sparkplug --no-maglev --no-lazy --liftoff-only --single-threaded --v8-pool-size=1 /opt/nexus/workbench/out/server-main.js --telemetry-level off --host 127.0.0.1 --port 18765 --connection-token \"$NEXUS_WORKBENCH_TOKEN\" --server-data-dir /root/nexus-vscode-data --extensions-dir /root/nexus-vscode-extensions /root/nexus-workbench";
fn fail(message: &str) -> Value { json!({"ok":false,"error":message}) }
fn valid_command(s: &str) -> bool { !s.trim().is_empty() && s.len() <= 2048 && !s.contains('\0') }
fn parse_call(input:&str)->Option<Value> {
    if input.len()>8192 {return None;}
    let value:Value=serde_json::from_str(input).ok()?;
    let obj=value.as_object()?;
    if obj.len()!=3 || !valid_command(obj.get("command")?.as_str()?) {return None;}
    let name=obj.get("name")?.as_str()?;
    if name.is_empty() || name.len()>128 || !name.bytes().all(|b|b.is_ascii_alphanumeric()||b"_.-".contains(&b)) {return None;}
    if !obj.get("arguments")?.is_object() || obj["arguments"].to_string().len()>4096 {return None;}
    Some(value)
}
// Hooked PTY jobs are separate from agent jobs and never receive provider keys.
fn parse_hooks(input:&str)->Option<Value> {
    if input.len()>8192 {return None;}
    let value:Value=serde_json::from_str(input).ok()?;
    let obj=value.as_object()?;
    if obj.len()!=3 || !valid_command(obj.get("command")?.as_str()?) {return None;}
    for field in ["before","after"] {
        let hook=obj.get(field)?;
        if !hook.is_null() && !hook.as_str().is_some_and(valid_command) {return None;}
    }
    if obj["before"].is_null() && obj["after"].is_null() {return None;}
    Some(value)
}
pub(crate) fn stop_requested()->bool {STATE.lock().map(|s|s.stop).unwrap_or(true)}
fn valid_token(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) }
fn parse_workbench(input:&str)->Option<(String,Option<String>)> {
    if valid_token(input) {return Some((input.to_owned(),None));}
    if input.len()>512 {return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    if v.as_object()?.len()!=2 {return None;}
    let token=v["token"].as_str()?.to_owned();
    let digest=v["vsixSha256"].as_str()?.to_owned();
    if !valid_token(&token) || !process_probe::agy_digest_ok(&digest) {return None;}
    Some((token,Some(digest)))
}
const INSTALL_VSIX:&str="umask 077; mkdir -p /root/nexus-vscode-data /root/nexus-vscode-extensions && exec /opt/nexus/workbench/node --no-opt --no-sparkplug --no-maglev --no-lazy --liftoff-only --single-threaded --v8-pool-size=1 /opt/nexus/workbench/out/server-main.js --telemetry-level off --server-data-dir /root/nexus-vscode-data --extensions-dir /root/nexus-vscode-extensions --install-extension /opt/nexus/extension.vsix";
fn parse_cli(input:&str)->Option<(String,String,String,Option<String>,Option<NativeSession>)> {
    if input.len()>16384 {return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    let fields=v.as_object()?.len();
    let kind=v["kind"].as_str()?;
    let digest=v["sha256"].as_str()?;
    let action=v["action"].as_str()?;
    if !matches!(kind,"codex"|"claude") || !process_probe::agy_digest_ok(digest) || !matches!(action,"shell"|"version"|"appServer"|"appServerChat"|"claudeAuth"|"claudeChat"|"codexSession"|"claudeSession") || (matches!(action,"appServer"|"appServerChat"|"codexSession") && kind!="codex") || (matches!(action,"claudeAuth"|"claudeChat"|"claudeSession")&&kind!="claude") {return None;}
    let interactive=matches!(action,"codexSession"|"claudeSession");
    let prompt=if interactive||matches!(action,"appServerChat"|"claudeChat") {
        let p=v["prompt"].as_str()?;
        if (if interactive{!matches!(fields,6|8)}else{fields!=4}) || !valid_agent_text(p){return None;}
        Some(p.to_owned())
    }else{if fields!=3{return None;}None};
    let session=if interactive{
        let retain=v["retain"].as_bool()?;
        let resume=v["resume"].as_str()?;
        if !resume.is_empty()&&(!retain||!valid_handle(resume)||(kind=="claude"&&!valid_uuid(resume))){return None;}
        let hook=|field:&str|->Option<Option<String>>{if fields==6{return Some(None);}let s=v.get(field)?.as_str()?;if s.is_empty(){Some(None)}else if valid_command(s){Some(Some(s.to_owned()))}else{None}};
        let before=hook("before")?;let after=hook("after")?;
        if fields==8&&before.is_none()&&after.is_none(){return None;}
        Some(NativeSession{retain,resume:if resume.is_empty(){None}else{Some(resume.to_owned())},before,after})
    }else{None};
    Some((kind.into(),digest.into(),action.into(),prompt,session))
}
pub(crate) struct AdapterInput {pub argv:Vec<String>,pub digest:String,pub prompt:String,pub resume:Option<String>}
fn parse_adapter(input:&str)->Option<AdapterInput>{
    if input.len()>8192{return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    let fields=v.as_object()?.len();
    if !matches!(fields,3|4)||!v["argv"].is_array(){return None;}
    let argv=process_probe::terminal_command(&v["argv"].to_string())?;
    let digest=v["sha256"].as_str()?;
    if !process_probe::agy_digest_ok(digest){return None;}
    let prompt=v["prompt"].as_str()?;
    if !valid_agent_text(prompt){return None;}
    let resume=if fields==4{let s=v["resume"].as_str()?;if !valid_handle(s){return None;}Some(s.to_owned())}else{None};
    Some(AdapterInput{argv,digest:digest.into(),prompt:prompt.into(),resume})
}
pub(crate) struct TransferInput {pub name:String,pub text:Option<String>}
struct BatchInput {push:bool,files:Vec<TransferInput>}
fn transfer_component(s:&str)->bool{!s.is_empty()&&s.len()<=128&&!s.starts_with('.')&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))}
fn parse_batch(input:&str)->Option<BatchInput>{
    if input.len()>262144{return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    if v.as_object()?.len()!=3{return None;}
    let push=match v["action"].as_str()?{"push"=>true,"pull"=>false,_=>return None};
    let id=v["batch"].as_str()?;if !transfer_component(id)||id.len()>64{return None;}
    let list=v["files"].as_array()?;if !(1..=8).contains(&list.len()){return None;}
    let mut files=Vec::<TransferInput>::new();let mut total=0;
    for f in list{
        let name=f["path"].as_str()?;
        if name.len()>256||name.split('/').count()>8||!name.split('/').all(transfer_component){return None;}
        let name=format!("batches/{id}/{name}");
        if files.iter().any(|x|x.name==name||x.name.starts_with(&format!("{name}/"))||name.starts_with(&format!("{}/",x.name))){return None;}
        let text=if push{if f.as_object()?.len()!=2{return None;}let t=f["text"].as_str()?;if t.len()>8192||t.contains('\0'){return None;}total+=t.len();if total>32768{return None;}Some(t.to_owned())}else{if f.as_object()?.len()!=1{return None;}None};
        files.push(TransferInput{name,text});
    }
    Some(BatchInput{push,files})
}
async fn run_batch(guest:&IshInstance,env:&HashMap<String,String>,batch:&BatchInput)->Value{
    let mut results=Vec::new();let mut total=0u64;
    for f in &batch.files{
        let r=process_probe::workspace_transfer(guest,env,f).await;
        if r["ok"]!=true{return json!({"ok":false,"completed":results.len(),"partialExportPossible":batch.push,"error":"Batch stopped; no native imports returned"});}
        total+=r["bytes"].as_u64().unwrap_or(32769);
        if total>32768{return json!({"ok":false,"completed":results.len(),"partialExportPossible":batch.push,"error":"Batch byte limit"});}
        results.push(r);
    }
    json!({"ok":true,"action":if batch.push{"push"}else{"pull"},"files":results,"bytes":total,"completed":results.len()})
}
fn parse_transfer(input:&str)->Option<TransferInput>{
    if input.len()>65536{return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    let name=v["name"].as_str()?;
    if name.is_empty()||name.len()>128||name.starts_with('.')||!name.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b)){return None;}
    let text=match v["action"].as_str()?{
        "push" if v.as_object()?.len()==3=>{let s=v["text"].as_str()?;if s.len()>8192||s.contains('\0'){return None;}Some(s.to_owned())},
        "pull" if v.as_object()?.len()==2=>None,
        _=>return None,
    };
    Some(TransferInput{name:name.into(),text})
}
pub(crate) struct LspInput {pub argv:Vec<String>,pub language:String,pub uri:String,pub text:String,pub completion:Option<(usize,usize)>}
fn parse_lsp(input:&str)->Option<LspInput>{
    if input.len()>65536{return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    let fields=v.as_object()?.len();
    if !matches!(fields,4|5){return None;}
    let argv=v["argv"].as_array()?;
    if !(1..=17).contains(&argv.len())||v["argv"].to_string().len()>2048{return None;}
    let argv=argv.iter().map(|a|a.as_str().filter(|s|s.len()<=512&&!s.chars().any(char::is_control)).map(str::to_owned)).collect::<Option<Vec<_>>>()?;
    let exe=argv.first()?;
    if exe.len()>256||!exe.starts_with('/')||!exe[1..].split('/').all(|p|!p.is_empty()&&p!="."&&p!=".."&&p.bytes().all(|b|b.is_ascii_alphanumeric()||b"._+-".contains(&b))){return None;}
    let language=v["language"].as_str()?;
    let ext=v["extension"].as_str()?;
    if language.is_empty()||language.len()>32||!language.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_".contains(&b))||ext.is_empty()||ext.len()>12||!ext.bytes().all(|b|b.is_ascii_alphanumeric()){return None;}
    let text=v["text"].as_str()?;
    if text.len()>8192||text.contains('\0'){return None;}
    let completion=if fields==5{
        let pos=v["completion"].as_object()?;
        if pos.len()!=2{return None;}
        let line=usize::try_from(pos.get("line")?.as_u64()?).ok()?;
        let col=usize::try_from(pos.get("character")?.as_u64()?).ok()?;
        let source=text.split('\n').nth(line)?.trim_end_matches('\r');
        let mut boundaries=std::iter::once(0).chain(source.chars().scan(0usize,|n,c|{*n+=c.len_utf16();Some(*n)}));
        if !boundaries.any(|n|n==col){return None;}
        Some((line,col))
    }else{None};
    Some(LspInput{argv,language:language.into(),uri:format!("file:///tmp/nexus-lsp.{ext}"),text:text.into(),completion})
}
fn progress() -> Value {
    STATE.lock().map(|s| json!({"ok":true,"running":s.running,"ready":s.ready,"output":s.output,"stopRequested":s.stop,"agentWaiting":s.agent.as_ref().is_some_and(|a|a.waiting&&!s.stop),"agentTurns":s.agent.as_ref().map(|a|&a.turns),"agentHandle":s.agent.as_ref().and_then(|a|a.handle.as_deref())}))
        .unwrap_or_else(|_| fail("Local status unavailable"))
}
pub(crate) fn publish_output(bytes: &[u8], ready: bool, truncated: bool, total: u64) {
    if let Ok(mut s) = STATE.lock() {
        // A plain-text PTY display, not an ANSI interpreter.
        if s.mode == 1 {
            s.raw.clear(); s.raw.extend_from_slice(bytes); s.total=total;
            s.output = String::from_utf8_lossy(bytes).chars()
                .map(|c| if c.is_control() && !matches!(c,'\n'|'\r'|'\t') {'�'} else {c}).collect();
            if truncated { s.output.insert_str(0,"[Older terminal output discarded]\n"); }
        }
        s.ready = ready && !s.stop;
    }
}
pub(crate) fn take_control() -> Vec<Control> {
    STATE.lock().map(|mut s| s.controls.drain(..).collect()).unwrap_or_default()
}
fn enqueue(control: Control) -> Value {
    let Ok(mut s) = STATE.lock() else { return fail("Local control unavailable"); };
    if !s.running || s.stop || matches!(s.mode,2|4) { return fail("No controllable local process"); }
    match control {
        Control::Stop => { s.stop = true; s.controls.clear(); s.controls.push_back(Control::Stop); }
        c if s.mode == 1 && s.ready && s.controls.len() < 8 => s.controls.push_back(c),
        _ => return fail("PTY not ready or queue full"),
    }
    json!({"ok":true})
}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut slot)=LIVE_LSP.lock(){*slot=None;}
        if let Ok(mut s) = STATE.lock() { s.running = false; s.ready = false; s.controls.clear(); if let Some(a)=s.agent.as_mut(){a.waiting=false;a.pending=None;} }
    }
}
// Explicit native conversation hooks only. No auto-discovery or native
// provider credential injection. A guest may already have private CLI data.
async fn run_agent_session(guest:&IshInstance,env:&HashMap<String,String>,kind:&str,digest:Option<&str>,prompt:&str,config:&NativeSession,fixture:bool)->Value{
    let before=match config.before.as_deref(){Some(cmd)=>Some(process_probe::local_hook(guest,env,cmd).await),None=>None};
    let executable=if kind=="codex"{"/opt/nexus/codex"}else{"/opt/nexus/claude"};
    let verified=if let Some(digest)=digest{process_probe::guest_file_digest(guest,env,executable,digest).await["passed"]==true}else{fixture};
    let mut r=if before.unwrap_or(true)&&verified&&!stop_requested(){
        if kind=="codex"{process_probe::codex_session(guest,env,prompt,config,fixture).await}
        else{process_probe::claude_session(guest,env,prompt,config,fixture).await}
    }else{fail("Before-session hook, executable verification or cancellation prevented start")};
    let after=if r["ok"]==true&&r["sessionFinished"]==true&&!stop_requested(){
        match config.after.as_deref(){Some(cmd)=>Some(process_probe::local_hook(guest,env,cmd).await),None=>None}
    }else{None};
    let verified_after=if let Some(digest)=digest{process_probe::guest_file_digest(guest,env,executable,digest).await["passed"]==true}else{fixture};
    r["hooks"]=json!({"before":before,"after":after,"afterSkipped":config.after.is_some()&&after.is_none()});
    r["binaryDigestVerified"]=json!(verified&&verified_after);
    r["ok"]=json!(r["ok"]==true&&after.unwrap_or(true)&&verified_after&&!stop_requested());
    r
}
// Diagnostic only. Never start a daemon, run an image, mount anything, or
// connect to a Docker socket. A positive primitive is not Engine support.
fn container_capabilities(guest:&IshInstance,env:&HashMap<String,String>)->Value{
    let tests=[
        ("namespaceEntries","test -e /proc/self/ns/user && test -e /proc/self/ns/mnt && test -e /proc/self/ns/pid && test -e /proc/self/ns/net"),
        ("userNamespace","command -v unshare >/dev/null && unshare --user --map-root-user /usr/bin/true"),
        ("mountNamespace","command -v unshare >/dev/null && unshare --mount /usr/bin/true"),
        ("cgroupV2Visible","test -r /sys/fs/cgroup/cgroup.controllers"),
        ("overlayListed","grep -qw overlay /proc/filesystems"),
        ("dockerCLIInstalled","command -v docker >/dev/null"),
        ("daemonSocketVisible","test -S /var/run/docker.sock"),
    ];
    let mut rows=serde_json::Map::new();
    for (name,script) in tests{
        if stop_requested(){return fail("Capability probe cancelled");}
        let argv=vec!["/bin/sh".into(),"-c".into(),script.into()];
        let (status,_)=guest.run_oneshot(&argv,Path::new("/"),env,Some(5000));
        rows.insert(name.into(),json!(status==0));
    }
    json!({"ok":true,"diagnosticOnly":true,"backend":"embedded-user-mode","capabilities":rows,"dockerEngineVerified":false,"composeVerified":false,"fullSystemVMVerified":false})
}
fn run(path: &Path, mode: u8, input: &str) -> Value {
    let call=if mode==4 {match parse_call(input) {Some(v)=>Some(v),None=>return fail("Invalid MCP call")}} else {None};
    let hooks=if mode==5 {match parse_hooks(input) {Some(v)=>Some(v),None=>return fail("Invalid hook plan")}} else {None};
    let workbench=if mode==3 {match parse_workbench(input) {Some(v)=>Some(v),None=>return fail("Invalid workbench launch")}} else {None};
    let cli=if mode==6 {match parse_cli(input) {Some(v)=>Some(v),None=>return fail("Invalid CLI launch")}} else {None};
    let lsp=if matches!(mode,8|12) {match parse_lsp(input){Some(v) if mode==8||v.completion.is_none()=>Some(v),_=>return fail("Invalid LSP snapshot request")}}else{None};
    let transfer=if mode==9 {match parse_transfer(input){Some(v)=>Some(v),None=>return fail("Invalid workspace transfer")}}else{None};
    let adapter=if mode==10 {match parse_adapter(input){Some(v)=>Some(v),None=>return fail("Invalid reviewed ACP adapter request")}}else{None};
    let batch=if mode==11 {match parse_batch(input){Some(v)=>Some(v),None=>return fail("Invalid batch transfer")}}else{None};
    if !matches!(mode,1..=6|8|9|10|11|12|13) || (mode==13&&input!="probe") ||
        (matches!(mode,1|2) && !valid_command(input)) { return fail("Invalid local job"); }
    if !path.is_dir() || !path.parent().is_some_and(|p| p.join("meta.db").is_file()) {
        return fail("Missing prepared fakefs");
    }
    if mode == 3 && (!path.join("opt/nexus/workbench/bin/openvscode-server").is_file() ||
        path.parent().and_then(|p| std::fs::read_to_string(p.join("workbench.sha256")).ok())
            .as_deref().map(str::trim) != Some("36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc")) {
        return fail("Reviewed workbench not staged");
    }
    let Ok(mut s) = STATE.lock() else { return fail("Local job lock failed"); };
    if s.running || ATTEMPTED.swap(true,Ordering::AcqRel) { return fail("Restart app before another guest boot"); }
    let native=adapter.is_some()||cli.as_ref().is_some_and(|c|c.4.is_some());
    *s = State { running:true,ready:false,mode:if native{7}else if matches!(mode,5|6) {1}else{mode},output:String::new(),raw:Vec::new(),total:0,controls:VecDeque::new(),stop:false,agent:if native{Some(AgentMailbox::default())}else{None} };
    drop(s);
    let _guard = Guard;
    if mode==12{if let Ok(mut slot)=LIVE_LSP.lock(){*slot=Some(LiveLsp::default());}else{return fail("LSP mailbox unavailable");}}
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt, Err(_) => return fail("Runtime creation failed; restart app"),
    };
    let guest = match IshInstance::boot(path,Some(Path::new("/"))) {
        Ok(g) => g, Err(_) => return fail("Guest boot failed"),
    };
    let mut env = HashMap::from([("PATH".into(),"/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ("HOME".into(),"/root".into()),("LANG".into(),"C.UTF-8".into()),
        ("TERM".into(),if matches!(mode,1|5|6) {"xterm-256color".into()} else {"dumb".into()}),("NODE_OPTIONS".into(),"--jitless".into())]);
    let libc = rt.block_on(process_probe::glibc_compat(&guest,&env));
    let mut result = if libc["passed"] != true { fail("Ubuntu compatibility failed") }
        else if mode==13 {container_capabilities(&guest,&env)}
        else if let Some(request)=adapter.as_ref(){rt.block_on(crate::persistent_acp::local_adapter_session(&guest,&env,request,false))}
        else if let Some(request)=batch.as_ref(){rt.block_on(run_batch(&guest,&env,request))}
        else if let Some(request)=transfer.as_ref(){rt.block_on(process_probe::workspace_transfer(&guest,&env,request))}
        else if let Some(request)=lsp.as_ref(){if mode==12{rt.block_on(process_probe::live_lsp(&guest,&env,request,false))}else{rt.block_on(process_probe::lsp_diagnostics(&guest,&env,request,false))}}
        else if let Some(plan)=hooks.as_ref() {
            let before=plan["before"].as_str();
            let after=plan["after"].as_str();
            let before_ok=before.is_none_or(|_|!stop_requested()) && match before {
                Some(cmd)=>rt.block_on(process_probe::local_hook(&guest,&env,cmd)),
                None=>true,
            };
            let mut result=if before_ok && !stop_requested() {
                rt.block_on(process_probe::local_process(&guest,&env,plan["command"].as_str().unwrap(),false))
            } else {fail("Before hook failed or job cancelled")};
            let main_ok=result["ok"]==true && result["exitCode"]==0 && result["stopped"]!=true && !stop_requested();
            let after_ok=if main_ok {
                match after {Some(cmd)=>Some(rt.block_on(process_probe::local_hook(&guest,&env,cmd))),None=>None}
            }else{None};
            result["hooks"]=json!({"before":if before.is_some(){Some(before_ok)}else{None},"after":after_ok,"afterSkipped":after.is_some()&&after_ok.is_none()});
            result["ok"]=json!(main_ok && after_ok.unwrap_or(true) && !stop_requested());
            result
        }
        else if let Some(call)=call.as_ref() {rt.block_on(process_probe::mcp_job(&guest,&env,call["command"].as_str().unwrap(),Some((call["name"].as_str().unwrap(),&call["arguments"]))))}
        else if mode == 2 { rt.block_on(process_probe::mcp_discover(&guest,&env,input)) }
        else if let Some((kind,digest,action,prompt,native_session))=cli.as_ref() {
            let executable=if kind=="codex" {"/opt/nexus/codex"}else{"/opt/nexus/claude"};
            let verified=rt.block_on(process_probe::guest_file_digest(&guest,&env,executable,digest))["passed"]==true;
            let mut r=if verified && !stop_requested() {
                // Only fixed paths/actions; no user-provided shell interpolation,
                // no CLI flags that disable the original harness safeguards.
                env.insert("PATH".into(),"/opt/nexus:/usr/bin:/bin:/usr/sbin:/sbin".into());
                if let Some(config)=native_session {
                    rt.block_on(run_agent_session(&guest,&env,kind,Some(digest),prompt.as_deref().unwrap_or(""),config,false))
                }
                else if matches!(action.as_str(),"appServer"|"appServerChat") {rt.block_on(process_probe::codex_app_server_probe(&guest,&env,false,prompt.as_deref()))}
                else if action=="claudeAuth"{rt.block_on(process_probe::claude_auth_probe(&guest,&env,false))}
                else if action=="claudeChat"{rt.block_on(process_probe::claude_text_turn(&guest,&env,prompt.as_deref().unwrap_or(""),false))}
                else {
                let script=match (kind.as_str(),action.as_str()) {
                    ("codex","version")=>"exec /opt/nexus/codex --version",
                    ("claude","version")=>"exec /opt/nexus/claude --version",
                    ("codex","shell")=>"umask 077; mkdir -p /root/nexus-cli-project && cd /root/nexus-cli-project && printf 'Reviewed Codex: use official login, then codex.\\n' && exec /bin/sh -i",
                    _=>"umask 077; mkdir -p /root/nexus-cli-project && cd /root/nexus-cli-project && printf 'Reviewed Claude Code: run claude and use its own login.\\n' && exec /bin/sh -i",
                };
                rt.block_on(process_probe::local_process(&guest,&env,script,false))
                }
            }else{fail("Reviewed CLI executable digest mismatch or cancelled")};
            if r.get("binaryDigestVerified").is_none(){r["binaryDigestVerified"]=json!(verified);}
            r["harness"]=json!(kind);
            r["action"]=json!(action);
            r
        }
        else if let Some((token,vsix))=workbench.as_ref() {
            // Only this reviewed Linux guest binary may use guest Wasm baseline
            // code generation. Asbestos interprets guest instructions; the host
            // backing pages are never executable. Do not enable host JIT.
            // V8 --jitless disables WebAssembly, breaking Node 22 Undici even
            // for MessageChannel. Keep JS optimization disabled for the main
            // entrypoints and permit guest Wasm Liftoff. These V8 flags cannot
            // be passed via NODE_OPTIONS; child guest processes have no host JIT.
            // The pinned litter-ish kernel/exec.c also injects --jitless
            // into every guest node exec, independently of NODE_OPTIONS.
            // Use its explicit per-environment compatibility opt-out only
            // in this reviewed Workbench lane; leave other guest lanes alone.
            // V8 background GC/compilation can race in ARM64 emulation.
            // Keep those tasks serial in the reviewed server and its inherited
            // execArgv (extension host). This does not disable Node async I/O
            // or guest Wasm, and never requests host executable memory.
            // Retain the upstream heap cap for Node descendants as well.
            env.insert("ISH_NODE_NO_ARG_INJECTION".into(),"1".into());
            env.insert("NODE_OPTIONS".into(),"--max-old-space-size=512".into());
            env.insert("UV_USE_IO_URING".into(),"0".into());
            let runtime=rt.block_on(process_probe::local_hook_with_timeout(&guest,&env,r#"
set -eu
test -r /opt/nexus/workbench/out/server-main.js
exec /opt/nexus/workbench/node --no-opt --no-sparkplug --no-maglev --no-lazy --liftoff-only --single-threaded --v8-pool-size=1 -e '
const fs = require("node:fs");
const { port1, port2 } = new MessageChannel();
const wasm = Uint8Array.from([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,7,1,3,114,117,110,0,0,10,6,1,4,0,65,42,11]);
if (new WebAssembly.Instance(new WebAssembly.Module(wasm)).exports.run() !== 42) throw Error("guest Wasm result");
const deadline = setTimeout(() => { console.error("guest MessageChannel timeout"); process.exit(1); }, 10000);
port1.addEventListener("message", event => {
    if (event.data !== 42) throw Error("guest MessageChannel result");
    port1.close(); port2.close(); clearTimeout(deadline);
    fs.stat("/tmp", (err, stat) => {
        if (err || !stat.isDirectory()) process.exitCode = 1;
        else console.log("NEXUS_NODE_WASM_MESSAGE_ASYNC_OK");
    });
});
port1.start(); port2.postMessage(42);
'
"#,60));
            let digest_verified=vsix.as_deref().map(|digest|
                rt.block_on(process_probe::guest_file_digest(&guest,&env,"/opt/nexus/extension.vsix",digest))["passed"]==true);
            #[cfg(test)]
            if std::env::var_os("NEXUS_WORKBENCH_CI_TRACE").is_some(){
                eprintln!("CI Workbench preflight: node={runtime}, vsix digest={digest_verified:?}");
            }
            let installed=vsix.as_deref().map(|_| {
                runtime && digest_verified==Some(true) && !stop_requested() && rt.block_on(process_probe::local_hook_with_timeout(&guest,&env,INSTALL_VSIX,120))
            });
            let mut r=if runtime && installed.unwrap_or(true) && !stop_requested() {
                env.insert("NEXUS_WORKBENCH_TOKEN".into(),token.clone());
                rt.block_on(process_probe::local_process(&guest,&env,WORKBENCH,true))
            } else {fail(if !runtime{"Workbench Node async preflight failed"}else if digest_verified==Some(false){"Reviewed VSIX digest mismatch"}else{"Reviewed VSIX installation failed, or launch cancelled"})};
            r["nodeRuntimeVerified"]=json!(runtime);
            r["extensionDigestVerified"]=json!(digest_verified);
            r["extensionInstalled"]=json!(installed);
            r
        } else {
            rt.block_on(process_probe::local_process(&guest,&env,input,false))
        };
    let shutdown = rt.block_on(guest.shutdown()).is_ok();
    result["shutdownClean"] = json!(shutdown);
    result["ok"] = json!(result["ok"] == true && shutdown);
    if mode==11 && result["ok"]!=true {result.as_object_mut().unwrap().remove("files");}
    if mode==9 && result["ok"]!=true {result.as_object_mut().unwrap().remove("text");}
    if mode==8 && result["ok"]!=true {let o=result.as_object_mut().unwrap();o.remove("diagnostics");o.remove("completions");}
    result
}
fn ffi(f: impl FnOnce() -> Value + std::panic::UnwindSafe) -> *mut c_char {
    let value = std::panic::catch_unwind(f).unwrap_or_else(|_| fail("Local runtime panic"));
    CString::new(value.to_string()).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}
unsafe fn arg(p:*const c_char) -> Option<String> {
    if p.is_null() { None } else { CStr::from_ptr(p).to_str().ok().map(str::to_owned) }
}
/// # Safety
/// Pointers must be valid NUL-terminated UTF-8 for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn nexus_local_run(path:*const c_char,mode:u8,input:*const c_char) -> *mut c_char {
    ffi(|| match (arg(path),arg(input)) { (Some(p),Some(i)) => run(Path::new(&p),mode,&i), _ => fail("Missing local arguments") })
}
#[no_mangle]
pub extern "C" fn nexus_local_progress() -> *mut c_char { ffi(progress) }
#[no_mangle]
pub extern "C" fn nexus_live_lsp_status()->*mut c_char{ffi(lsp_status)}
/// # Safety
/// text must be a NUL-terminated UTF-8 string for this call.
#[no_mangle]
pub unsafe extern "C" fn nexus_live_lsp_change(version:u32,text:*const c_char)->*mut c_char{ffi(||match arg(text){Some(t)=>lsp_control(version,Some(t)),None=>fail("Missing snapshot")})}
#[no_mangle]
pub extern "C" fn nexus_live_lsp_finish(version:u32)->*mut c_char{ffi(||lsp_control(version,None))}
/// # Safety
/// Pointer must be valid NUL-terminated UTF-8 for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn nexus_local_input(input:*const c_char) -> *mut c_char {
    ffi(|| match arg(input) {
        Some(s) if !s.is_empty() && s.len() <= 1024 => enqueue(Control::Input(s.as_bytes().to_vec())),
        _ => fail("Invalid PTY input"),
    })
}
fn terminal_delta(cursor:u64)->Value {
    let Ok(s)=STATE.lock() else {return fail("Terminal status unavailable");};
    if s.mode!=1 {return fail("No PTY");}
    let first=s.total.saturating_sub(s.raw.len() as u64);
    if cursor<first || cursor>s.total {
        return json!({"ok":false,"gap":true,"first":first,"next":s.total,"ready":s.ready});
    }
    let start=(cursor-first) as usize;
    json!({"ok":true,"gap":false,"next":s.total,"ready":s.ready,"running":s.running,"bytes":STANDARD.encode(&s.raw[start..])})
}
#[no_mangle]
pub extern "C" fn nexus_local_terminal_delta(cursor:u64)->*mut c_char {ffi(||terminal_delta(cursor))}
/// # Safety
/// Pointer must be valid NUL-terminated base64. Binary PTY input, no UTF-8 conversion.
#[no_mangle]
pub unsafe extern "C" fn nexus_local_terminal_input(input:*const c_char)->*mut c_char {
    ffi(|| match arg(input).filter(|s|s.len()<=1368).and_then(|s|STANDARD.decode(s).ok()) {
        Some(bytes) if !bytes.is_empty() && bytes.len()<=1024=>enqueue(Control::Input(bytes)),
        _=>fail("Invalid binary PTY input")
    })
}
#[no_mangle]
pub extern "C" fn nexus_local_resize(cols:u16,rows:u16) -> *mut c_char {
    ffi(|| if (20..=240).contains(&cols) && (5..=100).contains(&rows) {
        enqueue(Control::Resize(cols,rows))
    } else { fail("Invalid PTY size") })
}
/// # Safety
/// Valid NUL-terminated UTF-8 message. Never interpreted as terminal input.
#[no_mangle]
pub unsafe extern "C" fn nexus_local_agent_send(ticket:u32,text:*const c_char)->*mut c_char{
    ffi(||match arg(text){Some(s)=>agent_send(ticket,Some(s)),None=>fail("Missing message")})
}
#[no_mangle]
pub extern "C" fn nexus_local_agent_finish(ticket:u32)->*mut c_char{ffi(||agent_send(ticket,None))}
#[no_mangle]
pub extern "C" fn nexus_local_stop() -> *mut c_char { ffi(|| enqueue(Control::Stop)) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_input_bounds() {
        assert!(valid_command("printf ok"));
        assert!(!valid_command(" "));
        assert!(!valid_command("x\0y"));
        assert!(!valid_command(&"x".repeat(2049)));
        assert!(valid_token(&"a".repeat(64)));
        assert!(parse_cli(&json!({"kind":"codex","sha256":"a".repeat(64),"action":"shell"}).to_string()).is_some());
        assert!(parse_cli(&json!({"kind":"unknown","sha256":"a".repeat(64),"action":"shell"}).to_string()).is_none());
        assert!(parse_workbench(&json!({"token":"a".repeat(64),"vsixSha256":"b".repeat(64)}).to_string()).is_some());
        assert!(parse_workbench(&json!({"token":"a".repeat(64),"vsixSha256":"bad"}).to_string()).is_none());
        assert!(!valid_token("token"));
        assert!(parse_call(r#"{"command":"exec server","name":"echo","arguments":{}}"#).is_some());
        assert!(parse_call(r#"{"command":"exec server","name":"echo","arguments":[]}"#).is_none());
        assert!(parse_call(r#"{"command":"exec server","name":"echo","arguments":{},"extra":true}"#).is_none());
    }
    #[test]
    fn hooks_require_exact_explicit_plan() {
        assert!(parse_hooks(r#"{"command":"echo main","before":"echo pre","after":null}"#).is_some());
        assert!(parse_hooks(r#"{"command":"echo main","before":null,"after":null}"#).is_none());
        assert!(parse_hooks(r#"{"command":"echo main","before":" ","after":null}"#).is_none());
        assert!(parse_hooks(r#"{"command":"echo main","before":null,"after":"echo post","autoInstall":true}"#).is_none());
    }
    #[test]
    #[ignore = "requires fresh Ubuntu fakefs; synthetic Codex JSONL peer"]
    fn real_codex_app_server_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into())]);
        let mut r=rt.block_on(process_probe::codex_app_server_probe(&guest,&env,true,None));
        r["shutdownClean"]=json!(rt.block_on(guest.shutdown()).is_ok());
        r["fixtureOnly"]=json!(true);
        assert_eq!(r["ok"],true);
        assert_eq!(r["shutdownClean"],true);
        assert_eq!(r["accountPresent"],false);
        std::fs::write(report,serde_json::to_vec_pretty(&r).unwrap()).unwrap();
    }
    #[test]
    #[ignore = "requires fresh Ubuntu fakefs; synthetic Codex turn peer"]
    fn real_codex_native_turn_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_CODEX_TURN_PHASE").expect("phase");
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_CODEX_FIXTURE_TURN".into(),phase.clone())]);
        let r=rt.block_on(process_probe::codex_app_server_probe(&guest,&env,true,Some("Hello fixture")));
        let clean=rt.block_on(guest.shutdown()).is_ok();
        let ok=if phase=="success"{r["ok"]==true&&r["turnCompleted"]==true&&r["response"]=="Hello native"}else{r["ok"]==false&&r.get("response").is_none()};
        let evidence=json!({"ok":ok&&clean,"fixtureOnly":true,"phase":phase,"shutdownClean":clean});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        assert!(ok&&clean);
    }
    #[test]
    #[ignore = "requires independent Ubuntu guest boot for each phase"]
    fn real_native_conversation_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let kind=std::env::var("NEXUS_CONVERSATION_KIND").expect("kind");
        let phase=std::env::var("NEXUS_CONVERSATION_PHASE").expect("phase");
        let handle=if kind=="codex"{"fixture-thread"}else{"550e8400-e29b-41d4-a716-446655440000"};
        let config=NativeSession{retain:true,resume:if phase=="resume"{Some(handle.into())}else{None},before:Some("rm -f /tmp/nexus-agent-turn-done /tmp/nexus-agent-after; printf before >/tmp/nexus-agent-before".into()),after:Some("test \"$(cat /tmp/nexus-agent-turn-done)\" = done && printf after >/tmp/nexus-agent-after".into())};
        {
            let mut s=STATE.lock().unwrap();
            s.running=true;s.mode=7;s.stop=false;s.agent=Some(AgentMailbox::default());
        }
        let driver=std::thread::spawn(||{
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(30);
            let mut sent=false;
            while std::time::Instant::now()<deadline{
                let p=progress();
                if p["agentWaiting"]==true{
                    let n=p["agentTurns"].as_array().unwrap().len();
                    if n==1&&!sent{
                        assert_eq!(enqueue(Control::Input(b"forbidden".to_vec()))["ok"],false);
                        assert_eq!(agent_send(1,Some("Second fixture".into()))["ok"],true);
                        assert_eq!(agent_send(1,Some("duplicate".into()))["ok"],false);
                        sent=true;
                    }else if n==2{assert_eq!(agent_send(2,None)["ok"],true);return true;}
                }
                if p["running"]!=true{return false;}
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _=enqueue(Control::Stop);
            false
        });
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_CONVERSATION_PHASE".into(),phase.clone()),("NEXUS_CLAUDE_FIXTURE_PHASE".into(),"conversation".into())]);
        let r=rt.block_on(run_agent_session(&guest,&env,&kind,None,"First fixture",&config,true));
        let after_verified=rt.block_on(process_probe::local_hook(&guest,&env,"test \"$(cat /tmp/nexus-agent-after)\" = after"));
        let p=progress();
        let clean=rt.block_on(guest.shutdown()).is_ok();
        STATE.lock().unwrap().running=false;
        let driven=driver.join().unwrap();
        let ok=driven&&after_verified&&r["hooks"]["before"]==true&&r["hooks"]["after"]==true&&r["ok"]==true&&r["turnsCompleted"]==2&&p["agentHandle"]==handle&&p["agentTurns"][0]["response"]=="First reply"&&p["agentTurns"][1]["response"]=="Second reply"&&clean;
        let evidence=json!({"ok":ok,"fixtureOnly":true,"kind":kind,"phase":phase,"turnsCompleted":r["turnsCompleted"],"shutdownClean":clean});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        assert!(ok,"conversation fixture failed: {r}");
    }
    #[test]
    fn adapter_request_bounds(){
        let mut v=json!({"argv":["/usr/local/bin/claude-agent-acp"],"sha256":"a".repeat(64),"prompt":"hello"});
        assert!(parse_adapter(&v.to_string()).is_some());
        v["argv"]=json!(["../adapter"]);assert!(parse_adapter(&v.to_string()).is_none());
        v["argv"]=json!(["/usr/local/bin/claude-agent-acp"]);v["sha256"]=json!("bad");assert!(parse_adapter(&v.to_string()).is_none());
    }
    #[test]
    #[ignore = "requires fresh Ubuntu guest, synthetic ACP adapter"]
    fn real_local_adapter_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_ADAPTER_PHASE").expect("phase");
        let mut request=json!({"argv":["/usr/bin/true"],"sha256":"a".repeat(64),"prompt":"First adapter prompt"});
        if matches!(phase.as_str(),"resume"|"unsupportedResume"){request["resume"]=json!("adapter-fixture");}
        let input=parse_adapter(&request.to_string()).unwrap();
        {
            let mut s=STATE.lock().unwrap();s.running=true;s.stop=false;s.mode=7;s.agent=Some(AgentMailbox::default());
        }
        let driver=std::thread::spawn(||{
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(30);
            let mut sent=false;
            while std::time::Instant::now()<deadline{
                let p=progress();
                if p["agentWaiting"]==true{
                    let n=p["agentTurns"].as_array().unwrap().len();
                    if n==1&&!sent{assert_eq!(agent_send(1,Some("Second adapter prompt".into()))["ok"],true);sent=true;}
                    else if n==2{return agent_send(2,None)["ok"]==true;}
                }
                if p["running"]!=true{return false;}
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _=enqueue(Control::Stop);false
        });
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_ADAPTER_PHASE".into(),phase.clone())]);
        let r=rt.block_on(crate::persistent_acp::local_adapter_session(&guest,&env,&input,true));
        let p=progress();
        let clean=rt.block_on(guest.shutdown()).is_ok();
        STATE.lock().unwrap().running=false;
        let driven=driver.join().unwrap();
        let ok=if matches!(phase.as_str(),"success"|"resume"){driven&&r["ok"]==true&&r["turnsCompleted"]==2&&r["permissionDenials"]==1&&p["agentHandle"]=="adapter-fixture"&&p["agentTurns"][1]["response"]=="Second adapter reply"&&r["sessionLoaded"]==json!(phase=="resume")}else{r["ok"]==false&&p["agentTurns"].as_array().is_some_and(|a|a.is_empty())};
        std::fs::write(report,serde_json::to_vec_pretty(&json!({"ok":ok&&clean,"fixtureOnly":true,"phase":phase,"shutdownClean":clean})).unwrap()).unwrap();
        assert!(ok&&clean,"ACP adapter fixture failed: {r}");
    }
    #[test]
    #[ignore = "requires Ubuntu fakefs; reports only observed primitives"]
    fn real_container_capabilities(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let evidence=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let r=run(Path::new(&path),13,"probe");
        assert_eq!(r["ok"],true);
        assert_eq!(r["shutdownClean"],true);
        assert_eq!(r["diagnosticOnly"],true);
        assert_eq!(r["dockerEngineVerified"],false);
        assert_eq!(r["composeVerified"],false);
        assert_eq!(r["fullSystemVMVerified"],false);
        assert_eq!(r["capabilities"].as_object().unwrap().len(),7);
        std::fs::write(evidence,serde_json::to_vec_pretty(&r).unwrap()).unwrap();
    }
    #[test]
    fn batch_input_bounds(){
        let valid=json!({"action":"push","batch":"demo","files":[{"path":"src/a.txt","text":"hello"},{"path":"src/b.txt","text":""}]});
        assert!(parse_batch(&valid.to_string()).is_some());
        for path in ["../bad","src/.secret","src//bad","/root","src/a.txt/child","src/a.txt"]{
            let mut v=valid.clone();v["files"][1]["path"]=json!(path);assert!(parse_batch(&v.to_string()).is_none());
        }
    }
    #[test]
    #[ignore = "requires independent Ubuntu guest boots"]
    fn real_workspace_batch(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let evidence=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_BATCH_PHASE").expect("phase");
        let req=match phase.as_str(){
            "push"=>json!({"action":"push","batch":"fixture","files":[{"path":"src/a.txt","text":"Nexus Việt\n"},{"path":"docs/a.txt","text":"second"}]}),
            "duplicate"=>json!({"action":"push","batch":"fixture","files":[{"path":"src/a.txt","text":"no"}]}),
            "pull"=>json!({"action":"pull","batch":"fixture","files":[{"path":"src/a.txt"},{"path":"docs/a.txt"}]}),
            "missing"=>json!({"action":"pull","batch":"fixture","files":[{"path":"src/a.txt"},{"path":"missing"}]}),
            _=>panic!("unknown phase"),
        };
        let r=run(Path::new(&path),11,&req.to_string());
        let ok=match phase.as_str(){
            "push"=>r["ok"]==true&&r["completed"]==2,
            "pull"=>r["ok"]==true&&r["files"][0]["text"]=="Nexus Việt\n"&&r["files"][1]["text"]=="second",
            _=>r["ok"]==false&&r.get("files").is_none(),
        }&&r["shutdownClean"]==true;
        std::fs::write(evidence,serde_json::to_vec_pretty(&json!({"ok":ok,"phase":phase,"fixtureOnly":true,"shutdownClean":r["shutdownClean"]})).unwrap()).unwrap();
        assert!(ok,"batch failed: {}",r["error"]);
    }
    #[test]
    fn transfer_input_bounds(){
        assert!(parse_transfer(r#"{"action":"push","name":"demo.txt","text":""}"#).is_some());
        for name in ["../demo",".hidden","/root/a","a/b","a b"]{
            assert!(parse_transfer(&json!({"action":"pull","name":name}).to_string()).is_none());
        }
        assert!(parse_transfer(&json!({"action":"push","name":"a","text":"x".repeat(8193)}).to_string()).is_none());
    }
    #[test]
    #[ignore = "requires independent boots sharing the same Ubuntu fakefs"]
    fn real_workspace_transfer(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let evidence=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_TRANSFER_PHASE").expect("phase");
        let text="Nexus — Tiếng Việt\n$(never-execute)\n";
        let request=match phase.as_str(){
            "push"=>json!({"action":"push","name":"demo.txt","text":text}),
            "duplicate"=>json!({"action":"push","name":"demo.txt","text":"must not overwrite"}),
            "pull"=>json!({"action":"pull","name":"demo.txt"}),
            _=>panic!("unknown phase"),
        };
        let r=run(Path::new(&path),9,&request.to_string());
        let ok=match phase.as_str(){
            "push"=>r["ok"]==true&&r["bytes"]==text.len(),
            "duplicate"=>r["ok"]==false&&r.get("text").is_none(),
            _=>r["ok"]==true&&r["text"]==text,
        }&&r["shutdownClean"]==true;
        std::fs::write(evidence,serde_json::to_vec_pretty(&json!({"ok":ok,"phase":phase,"shutdownClean":r["shutdownClean"],"fixtureOnly":true})).unwrap()).unwrap();
        assert!(ok,"transfer failed: {}",r["error"]);
    }
    #[test]
    #[ignore = "requires independent Ubuntu boot"]
    fn real_live_lsp_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let evidence=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_LIVE_LSP_PHASE").expect("phase");
        let input=parse_lsp(&json!({"argv":["/usr/bin/server"],"language":"python","extension":"py","text":"x = 1\n"}).to_string()).unwrap();
        *LIVE_LSP.lock().unwrap()=Some(LiveLsp::default());
        {let mut s=STATE.lock().unwrap();s.running=true;s.stop=false;s.mode=12;}
        let driver=std::thread::spawn(||{
            let until=std::time::Instant::now()+std::time::Duration::from_secs(30);
            let mut sent=false;
            while std::time::Instant::now()<until{
                let p=lsp_status();
                if p["waiting"]==true&&p["version"]==1&&!sent{
                    assert_eq!(lsp_control(0,Some("stale".into()))["ok"],false);
                    assert_eq!(lsp_control(1,Some("x = 2\n".into()))["ok"],true);
                    assert_eq!(lsp_control(1,Some("duplicate".into()))["ok"],false);sent=true;
                }else if p["waiting"]==true&&p["version"]==2{
                    return (sent,p["diagnostics"][0]["message"]=="version two"&&lsp_control(2,None)["ok"]==true);
                }
                if !STATE.lock().unwrap().running{return (sent,false);}
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _=enqueue(Control::Stop);(sent,false)
        });
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_LIVE_LSP_PHASE".into(),phase.clone())]);
        let r=rt.block_on(process_probe::live_lsp(&guest,&env,&input,true));
        let clean=rt.block_on(guest.shutdown()).is_ok();
        STATE.lock().unwrap().running=false;
        let (sent,driven)=driver.join().unwrap();
        *LIVE_LSP.lock().unwrap()=None;
        let ok=clean&&sent&&r["transportClosed"]==true&&match phase.as_str(){
            "success"=>driven&&r["ok"]==true&&r["versions"]==2,
            "future"=>r["ok"]==false&&!driven&&r["fixtureError"]=="LSP diagnostic document mismatch",
            _=>false,
        };
        std::fs::write(evidence,serde_json::to_vec_pretty(&json!({"ok":ok,"phase":phase,"fixtureOnly":true,"shutdownClean":clean})).unwrap()).unwrap();
        assert!(ok,"live LSP fixture failed: {r}");
    }
    #[test]
    fn lsp_input_bounds(){
        let valid=json!({"argv":["/usr/bin/server","--stdio"],"language":"python","extension":"py","text":"x = 1"});
        assert!(parse_lsp(&valid.to_string()).is_some());
        let mut v=valid.clone();v["argv"]=json!(["../../server"]);assert!(parse_lsp(&v.to_string()).is_none());
        let mut v=valid.clone();v["extension"]=json!("../py");assert!(parse_lsp(&v.to_string()).is_none());
        let mut v=valid.clone();v["text"]=json!("x".repeat(8193));assert!(parse_lsp(&v.to_string()).is_none());
    }
    #[test]
    #[ignore = "requires prepared real pylsp ARM64 Ubuntu fakefs"]
    fn real_pylsp_snapshot(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let evidence=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin:/usr/sbin:/sbin".into()),("HOME".into(),"/root".into()),("LANG".into(),"C.UTF-8".into()),("PYTHONFAULTHANDLER".into(),"1".into())]);
        {let mut s=STATE.lock().unwrap();s.running=true;s.stop=false;s.mode=8;}
        let version=rt.block_on(process_probe::verify_pylsp_for_ci(&guest,&env));
        let threads=version.is_ok()&&rt.block_on(process_probe::verify_python_threads_for_ci(&guest,&env));
        let r=if threads{
            let input=parse_lsp(&json!({"argv":["/usr/bin/python3","-u","-m","pylsp","-v"],"language":"python","extension":"py","text":"x = missing_nexus_symbol"}).to_string()).unwrap();
            rt.block_on(process_probe::lsp_diagnostics(&guest,&env,&input,false))
        }else{json!({"ok":false,"error":"Prepared server or Python thread regression failed"})};
        let clean=rt.block_on(guest.shutdown()).is_ok();
        STATE.lock().unwrap().running=false;
        let diagnosed=r["diagnostics"].as_array().is_some_and(|a|a.iter().any(|d|d["message"].as_str().is_some_and(|m|m.contains("missing_nexus_symbol"))&&d["range"]["start"]["line"]==0&&d["range"]["start"]["character"]==4));
        let ok=version.as_deref()==Ok("1.10.0-1")&&threads&&r["ok"]==true&&r["transportClosed"]==true&&diagnosed&&clean;
        let report=json!({"ok":ok,"fixtureOnly":false,"scope":"macOS-hosted Ubuntu guest","package":"python3-pylsp","packageVersion":version.ok(),"pythonThreadsVerified":threads,"diagnosticVerified":diagnosed,"transportClosed":r["transportClosed"],"shutdownClean":clean,"physicalDeviceVerified":false});
        std::fs::write(evidence,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        assert!(ok,"real pylsp snapshot gate: {report}; {}",r["error"]);
    }
    #[test]
    #[ignore = "requires separate Ubuntu fakefs"]
    fn real_lsp_diagnostic_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_LSP_PHASE").expect("phase");
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_LSP_PHASE".into(),phase.clone())]);
        let mut request=json!({"argv":["/usr/bin/server"],"language":"python","extension":"py","text":"x = 1\n"});
        if phase.starts_with("completion"){request["completion"]=json!({"line":0,"character":5});}
        let input=parse_lsp(&request.to_string()).unwrap();
        let r=rt.block_on(process_probe::lsp_diagnostics(&guest,&env,&input,true));
        let clean=rt.block_on(guest.shutdown()).is_ok();
        let ok=if phase=="success"{r["ok"]==true&&r["diagnostics"][0]["message"]=="fixture diagnostic"&&r["diagnostics"][0]["range"]["end"]["character"]==1}else if phase=="completion"{r["ok"]==true&&r["completions"].as_array().is_some_and(|a|a.len()==1)&&r["completions"][0]["text"]==" + 2"}else{r["ok"]==false&&r.get("diagnostics").is_none()&&r.get("completions").is_none()};
        let evidence=json!({"ok":ok&&clean,"fixtureOnly":true,"phase":phase,"shutdownClean":clean});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        assert!(ok&&clean,"LSP fixture failed: {r}");
    }
    #[test]
    fn native_session_request_bounds(){
        let valid=json!({"kind":"codex","sha256":"a".repeat(64),"action":"codexSession","prompt":"hello","retain":true,"resume":"fixture-thread"});
        assert!(parse_cli(&valid.to_string()).is_some());
        let mut v=valid.clone();v["retain"]=json!(false);assert!(parse_cli(&v.to_string()).is_none());
        let mut v=valid.clone();v["resume"]=json!("../../bad");assert!(parse_cli(&v.to_string()).is_none());
        let mut v=valid.clone();v["prompt"]=json!("x".repeat(2049));assert!(parse_cli(&v.to_string()).is_none());
        let mut v=valid.clone();v["before"]=json!("echo before");v["after"]=json!("echo after");assert!(parse_cli(&v.to_string()).is_some());
        v["before"]=json!(" ");assert!(parse_cli(&v.to_string()).is_none());
        assert!(valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!valid_uuid("not-a-session-id"));
    }
    #[test]
    #[ignore = "requires fresh Ubuntu fakefs; synthetic Claude JSON peer"]
    fn real_claude_native_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_CLAUDE_FIXTURE_PHASE").expect("phase");
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_CLAUDE_FIXTURE_PHASE".into(),phase.clone())]);
        let r=if phase=="signedIn"||phase=="signedOut"{rt.block_on(process_probe::claude_auth_probe(&guest,&env,true))}else{rt.block_on(process_probe::claude_text_turn(&guest,&env,"Hello fixture",true))};
        let clean=rt.block_on(guest.shutdown()).is_ok();
        let ok=match phase.as_str(){
            "signedIn"=>r["ok"]==true&&r["accountPresent"]==true&&!r.to_string().contains("fixture-private"),
            "signedOut"=>r["ok"]==true&&r["accountPresent"]==false,
            "success"=>r["ok"]==true&&r["turnCompleted"]==true&&r["response"]=="Hello Claude",
            _=>r["ok"]==false&&r.get("response").is_none(),
        };
        let evidence=json!({"ok":ok&&clean,"fixtureOnly":true,"phase":phase,"shutdownClean":clean});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        assert!(ok&&clean);
    }
    #[test]
    #[ignore = "requires fresh Ubuntu fakefs; real Ubuntu ELF, not actual agent"]
    fn real_cli_import_slot() {
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let kind=std::env::var("NEXUS_CLI_KIND").expect("kind");
        let digest=std::env::var("NEXUS_CLI_SHA").expect("sha");
        let input=json!({"kind":kind,"sha256":digest,"action":"version"}).to_string();
        let result=run(Path::new(&path),6,&input);
        let output=progress()["output"].as_str().unwrap_or("").to_owned();
        let verified=result["ok"]==true && result["binaryDigestVerified"]==true && result["exitCode"]==0 && output.contains("GNU coreutils");
        let evidence=json!({"ok":verified && result["shutdownClean"]==true,"fixture":"Ubuntu true, not Codex or Claude","kind":kind,"shutdownClean":result["shutdownClean"]});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        println!("{evidence}");
        assert_eq!(evidence["ok"],true);
    }
    #[test]
    #[ignore = "requires fresh Ubuntu fakefs in separate process"]
    fn real_local_hooks() {
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_HOOK_PHASE").expect("phase");
        let plan=match phase.as_str() {
            "success"=>json!({"before":"printf pre >/tmp/nexus-hook-state","command":"test \"$(cat /tmp/nexus-hook-state)\" = pre && printf main >/tmp/nexus-hook-state && printf 'NEXUS_HOOK_MAIN\\n'","after":"test \"$(cat /tmp/nexus-hook-state)\" = main"}),
            "failure"=>json!({"before":"exit 7","command":"printf NEXUS_FORBIDDEN_MAIN","after":"exit 0"}),
            "main_failure"=>json!({"before":"true","command":"exit 7","after":"exit 0"}),
            _=>panic!("invalid hook phase"),
        };
        let result=run(Path::new(&path),5,&plan.to_string());
        let output=progress()["output"].as_str().unwrap_or("").to_owned();
        let verified=match phase.as_str() {
            "success"=>result["ok"]==true && result["hooks"]["before"]==true && result["hooks"]["after"]==true && output.contains("NEXUS_HOOK_MAIN"),
            "failure"=>result["ok"]==false && result["hooks"]["before"]==false && result["hooks"]["afterSkipped"]==true && !output.contains("NEXUS_FORBIDDEN_MAIN"),
            "main_failure"=>result["ok"]==false && result["exitCode"]==7 && result["hooks"]["before"]==true && result["hooks"]["afterSkipped"]==true,
            _=>false
        };
        let evidence=json!({"ok":verified && result["shutdownClean"]==true,"phase":phase,"shutdownClean":result["shutdownClean"]});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        println!("{evidence}");
        assert_eq!(evidence["ok"],true);
    }
    #[test]
    #[ignore = "requires reviewed real OpenVSCode rootfs; loopback host probe"]
    fn real_workbench_loopback() {
        use std::io::{Read,Write};
        use std::net::{TcpStream,SocketAddr};
        use std::time::{Duration,Instant};
        let path=std::env::var_os("NEXUS_WORKBENCH_ROOTFS").expect("rootfs");
        let evidence=std::env::var_os("NEXUS_WORKBENCH_EVIDENCE").expect("evidence");
        // Synthetic CI-only token. No real account or native workspace.
        let token="f".repeat(64);
        let vsix=std::env::var("NEXUS_WORKBENCH_VSIX_SHA").ok();
        let worker_token=match vsix.as_deref() {
            Some(digest)=>json!({"token":token,"vsixSha256":digest}).to_string(),
            None=>token.clone(),
        };
        let guest_path=Path::new(&path).to_path_buf();
        let worker=std::thread::spawn(move||run(Path::new(&path),3,&worker_token));
        let deadline=Instant::now()+Duration::from_secs(240);
        let ready=loop {
            if progress()["ready"]==true {break true;}
            if worker.is_finished() || Instant::now()>=deadline {break false;}
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut status=None;
        if ready {
            let addr:SocketAddr="127.0.0.1:18765".parse().unwrap();
            if let Ok(mut stream)=TcpStream::connect_timeout(&addr,Duration::from_secs(5)) {
                let _=stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _=stream.set_write_timeout(Some(Duration::from_secs(5)));
                let request=format!("GET /?tkn={token} HTTP/1.1\r\nHost: 127.0.0.1:18765\r\nConnection: close\r\n\r\n");
                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut buf=[0u8;1024];
                    let mut header=Vec::new();
                    while header.len()<8192 {
                        match stream.read(&mut buf) {
                            Ok(n) if n>0 => {header.extend_from_slice(&buf[..n]); if header.windows(2).any(|s|s==b"\r\n"){break;}}
                            _=>break,
                        }
                    }
                    status=String::from_utf8_lossy(&header).lines().next().and_then(|s| {
                        let mut parts=s.split_ascii_whitespace();
                        if parts.next()!=Some("HTTP/1.1") {return None;}
                        parts.next()?.parse::<u16>().ok()
                    });
                }
            }
        }
        let browser_requested=std::env::var_os("NEXUS_WORKBENCH_BROWSER_PYTHON").is_some();
        let browser_ok=if ready&&matches!(status,Some(200|302|303|307))&&browser_requested&&vsix.is_some(){
            // Fixed synthetic local browser probe. No user data or real credentials.
            std::process::Command::new(std::env::var_os("NEXUS_WORKBENCH_BROWSER_PYTHON").unwrap())
                .arg(std::env::var_os("NEXUS_WORKBENCH_BROWSER_SCRIPT").expect("browser script"))
                .env("NEXUS_WORKBENCH_TEST_TOKEN",&token)
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit())
                .status().is_ok_and(|s|s.success())
        }else{false};
        eprintln!("CI Workbench: requesting stop; browser_ok={browser_ok}");
        let stop=enqueue(Control::Stop);
        let stop_deadline=Instant::now()+Duration::from_secs(90);
        while !worker.is_finished() && Instant::now()<stop_deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if !worker.is_finished() {
            let report=json!({"ok":false,"ready":ready,"httpStatus":status,"shutdownClean":false,
                "browserRequested":browser_requested,"browserCompleted":browser_ok,
                "error":"Workbench runtime did not exit within stop watchdog"});
            std::fs::write(&evidence,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            eprintln!("{report}");
            // CI-only isolated test executable. Never continue with a stuck
            // guest or claim a clean shutdown, even if editor bytes exist.
            std::process::exit(1);
        }
        let closed=worker.join().expect("workbench owner");
        // CI-only fixed synthetic input; returned status contains no server URL or token.
        eprintln!("CI Workbench final process status: {closed}");
        let extension_verified=vsix.as_ref().map(|_| {
            closed["extensionInstalled"]==true &&
                std::fs::read_dir(guest_path.join("root/nexus-vscode-extensions")).ok().is_some_and(|dirs|
                    dirs.filter_map(Result::ok).any(|entry| {
                        let name=entry.file_name();
                        name.to_string_lossy().starts_with("nexus.fixture-0.0.1") &&
                            entry.path().join("package.json").is_file()
                    }))
        });
        let root=guest_path.join("root/nexus-workbench");
        let activated=browser_ok&&std::fs::read(root.join("nexus-extension-activated.marker")).ok().as_deref()==Some(b"activated");
        let editor_verified=activated&&std::fs::read(root.join("nexus-editor-saved.marker")).ok().as_deref()==Some(b"saved");
        let lsp_verified=editor_verified&&std::fs::read(root.join("nexus-lsp-transport.marker")).ok().as_deref()==Some(b"completed")&&std::fs::read(root.join("nexus-lsp-saved.marker")).ok().as_deref()==Some(b"saved")&&std::fs::read(root.join("nexus-editor-fixture.txt")).ok().as_deref()==Some(b"NEXUS_BROWSER_EDITOR_SAVED\nNEXUS_LSP_COMPLETION");
        let report=json!({"ok":ready && matches!(status,Some(200|302|303|307)) && stop["ok"]==true && closed["ok"]==true && closed["shutdownClean"]==true && extension_verified.unwrap_or(true)&&(!browser_requested||(activated&&editor_verified&&lsp_verified)),
            "ready":ready,"httpStatus":status,"shutdownClean":closed["shutdownClean"],"nodeRuntimeVerified":closed["nodeRuntimeVerified"],"extensionDigestVerified":closed["extensionDigestVerified"],"extensionInstallVerified":extension_verified,"browserRequested":browser_requested,"extensionActivationVerified":activated,"editorVerified":editor_verified,"lspVerified":lsp_verified,"lspFixtureOnly":true});
        std::fs::write(evidence,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{report}");
        assert_eq!(report["ok"],true);
    }
    #[test]
    #[ignore = "requires separate process per persisted guest boot"]
    fn real_persistent_local_guest() {
        let path=std::env::var_os("NEXUS_PERSIST_ROOTFS").expect("rootfs");
        let phase=std::env::var("NEXUS_PERSIST_PHASE").expect("phase");
        let cmd=match phase.as_str() {
            "write"=>"mkdir -p /root/nexus-test && printf 42 >/root/nexus-test/value && printf 'NEXUS_PERSIST_WRITTEN\\n'",
            "read"=>"test \"$(cat /root/nexus-test/value)\" = 42 && printf 'NEXUS_PERSIST_READ\\n'",
            "scroll"=>"head -c 100000 /dev/zero | tr '\\000' x; printf '\\nNEXUS_SCROLL_TAIL\\n'",
            _=>panic!("invalid phase"),
        };
        let mut result=run(Path::new(&path),1,cmd);
        let output=progress()["output"].as_str().unwrap_or("").to_owned();
        let expected=match phase.as_str() {"write"=>"NEXUS_PERSIST_WRITTEN","read"=>"NEXUS_PERSIST_READ",_=>"NEXUS_SCROLL_TAIL"};
        result["verified"]=json!(output.contains(expected) && (phase!="scroll" || (result["outputTruncated"]==true && output.contains("[Older terminal output discarded]") && terminal_delta(0)["gap"]==true)));
        result["ok"]=json!(result["ok"]==true && result["verified"]==true && result["exitCode"]==0);
        println!("{result}");
        std::fs::write(std::env::var_os("NEXUS_PERSIST_EVIDENCE").expect("evidence"),serde_json::to_vec_pretty(&result).unwrap()).unwrap();
        assert_eq!(result["ok"],true);
    }
    #[test]
    #[ignore = "requires fresh Ubuntu fakefs in a separate process"]
    fn real_local_job() {
        let path = std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("local rootfs");
        let report = std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("local evidence");
        let mcp = std::env::var("NEXUS_LOCAL_MCP").as_deref() == Ok("1");
        let call = std::env::var("NEXUS_LOCAL_CALL").as_deref() == Ok("1");
        let result = if call {
            let cmd=format!("NEXUS_FIXTURE_CALL=1; export NEXUS_FIXTURE_CALL\n{}",include_str!("../fixtures/mcp-discovery.sh"));
            let wire=json!({"command":cmd,"name":"fixture_echo","arguments":{"value":42}}).to_string();
            run(Path::new(&path),4,&wire)
        } else if mcp {
            run(Path::new(&path),2,include_str!("../fixtures/mcp-discovery.sh"))
        } else {
            let worker = std::thread::spawn(move || run(Path::new(&path),1,
                "printf 'NEXUS_LOCAL_READY\\n'; IFS= read -r line; stty size; printf 'INPUT:%s\\n' \"$line\""));
            let deadline = std::time::Instant::now()+std::time::Duration::from_secs(15);
            let ready = loop {
                let p = progress();
                if p["ready"] == true && p["output"].as_str().unwrap_or("").contains("NEXUS_LOCAL_READY") { break true; }
                if worker.is_finished() || std::time::Instant::now() >= deadline { break false; }
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            let resized = enqueue(Control::Resize(111,33));
            let input = enqueue(Control::Input(b"nexus-input\n".to_vec()));
            let mut r = worker.join().expect("local worker");
            let output = progress()["output"].as_str().unwrap_or("").to_owned();
            let delta=terminal_delta(0);
            let bytes=STANDARD.decode(delta["bytes"].as_str().unwrap_or("")).unwrap();
            let next=delta["next"].as_u64().unwrap_or(0);
            let no_replay=terminal_delta(next);
            r["rawPtyVerified"]=json!(delta["ok"]==true && next==bytes.len() as u64 &&
                bytes.windows(b"INPUT:nexus-input".len()).any(|s|s==b"INPUT:nexus-input") &&
                no_replay["ok"]==true && no_replay["bytes"]=="");
            r["ptyVerified"] = json!(ready && resized["ok"] == true && input["ok"] == true &&
                output.contains("33 111") && output.contains("INPUT:nexus-input") && r["rawPtyVerified"]==true);
            r["ok"] = json!(r["ok"] == true && r["ptyVerified"] == true && r["exitCode"] == 0);
            r
        };
        std::fs::write(report,serde_json::to_vec_pretty(&result).unwrap()).expect("completed local evidence");
        println!("{result}");
        assert_eq!(result["ok"],true);
        assert_eq!(result["shutdownClean"],true);
        if mcp || call { assert_eq!(result["tools"],json!(["fixture_echo"])); }
        if call { assert_eq!(result["calledTool"],"fixture_echo"); assert_eq!(result["text"],"NEXUS_MCP_42"); assert_eq!(result["discoveryOnly"],false); }
    }
}
