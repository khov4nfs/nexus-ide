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
pub(crate) struct NativeSession {pub retain:bool,pub resume:Option<String>}
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
fn agent_send(text:Option<String>)->Value{
    let Ok(mut s)=STATE.lock()else{return fail("Agent status unavailable");};
    if !s.running||s.stop{return fail("No active agent session");}
    let Some(a)=s.agent.as_mut()else{return fail("No native agent session");};
    if !a.waiting||a.pending.is_some()||a.finish{return fail("Agent is not waiting");}
    if let Some(text)=text{
        if !valid_agent_text(&text){return fail("Invalid message");}
        a.pending=Some(text);
    }else{a.finish=true;}
    a.waiting=false;
    json!({"ok":true})
}
struct State {
    running: bool, ready: bool, mode: u8, output: String, raw: Vec<u8>, total: u64,
    controls: VecDeque<Control>, stop: bool, agent:Option<AgentMailbox>,
}
static STATE: Mutex<State> = Mutex::new(State {
    running:false, ready:false, mode:0, output:String::new(), raw:Vec::new(), total:0, controls:VecDeque::new(), stop:false, agent:None,
});
const WORKBENCH: &str = "umask 077; mkdir -p /root/nexus-workbench /root/nexus-vscode-data /root/nexus-vscode-extensions && exec /opt/nexus/workbench/bin/openvscode-server --host 127.0.0.1 --port 18765 --connection-token \"$NEXUS_WORKBENCH_TOKEN\" --server-data-dir /root/nexus-vscode-data --extensions-dir /root/nexus-vscode-extensions /root/nexus-workbench";
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
const INSTALL_VSIX:&str="umask 077; mkdir -p /root/nexus-vscode-data /root/nexus-vscode-extensions && exec /opt/nexus/workbench/bin/openvscode-server --server-data-dir /root/nexus-vscode-data --extensions-dir /root/nexus-vscode-extensions --install-extension /opt/nexus/extension.vsix";
fn parse_cli(input:&str)->Option<(String,String,String,Option<String>,Option<NativeSession>)> {
    if input.len()>8192 {return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    let fields=v.as_object()?.len();
    let kind=v["kind"].as_str()?;
    let digest=v["sha256"].as_str()?;
    let action=v["action"].as_str()?;
    if !matches!(kind,"codex"|"claude") || !process_probe::agy_digest_ok(digest) || !matches!(action,"shell"|"version"|"appServer"|"appServerChat"|"claudeAuth"|"claudeChat"|"codexSession"|"claudeSession") || (matches!(action,"appServer"|"appServerChat"|"codexSession") && kind!="codex") || (matches!(action,"claudeAuth"|"claudeChat"|"claudeSession")&&kind!="claude") {return None;}
    let interactive=matches!(action,"codexSession"|"claudeSession");
    let prompt=if interactive||matches!(action,"appServerChat"|"claudeChat") {
        let p=v["prompt"].as_str()?;
        if fields!=(if interactive{6}else{4}) || !valid_agent_text(p){return None;}
        Some(p.to_owned())
    }else{if fields!=3{return None;}None};
    let session=if interactive{
        let retain=v["retain"].as_bool()?;
        let resume=v["resume"].as_str()?;
        if !resume.is_empty()&&(!retain||!valid_handle(resume)||(kind=="claude"&&!valid_uuid(resume))){return None;}
        Some(NativeSession{retain,resume:if resume.is_empty(){None}else{Some(resume.to_owned())}})
    }else{None};
    Some((kind.into(),digest.into(),action.into(),prompt,session))
}
pub(crate) struct LspInput {pub argv:Vec<String>,pub language:String,pub uri:String,pub text:String}
fn parse_lsp(input:&str)->Option<LspInput>{
    if input.len()>65536{return None;}
    let v:Value=serde_json::from_str(input).ok()?;
    if v.as_object()?.len()!=4{return None;}
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
    Some(LspInput{argv,language:language.into(),uri:format!("file:///tmp/nexus-lsp.{ext}"),text:text.into()})
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
        if let Ok(mut s) = STATE.lock() { s.running = false; s.ready = false; s.controls.clear(); if let Some(a)=s.agent.as_mut(){a.waiting=false;a.pending=None;} }
    }
}
fn run(path: &Path, mode: u8, input: &str) -> Value {
    let call=if mode==4 {match parse_call(input) {Some(v)=>Some(v),None=>return fail("Invalid MCP call")}} else {None};
    let hooks=if mode==5 {match parse_hooks(input) {Some(v)=>Some(v),None=>return fail("Invalid hook plan")}} else {None};
    let workbench=if mode==3 {match parse_workbench(input) {Some(v)=>Some(v),None=>return fail("Invalid workbench launch")}} else {None};
    let cli=if mode==6 {match parse_cli(input) {Some(v)=>Some(v),None=>return fail("Invalid CLI launch")}} else {None};
    let lsp=if mode==8 {match parse_lsp(input){Some(v)=>Some(v),None=>return fail("Invalid LSP snapshot request")}}else{None};
    if !matches!(mode,1..=6|8) ||
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
    let native=cli.as_ref().is_some_and(|c|c.4.is_some());
    *s = State { running:true,ready:false,mode:if native{7}else if matches!(mode,5|6) {1}else{mode},output:String::new(),raw:Vec::new(),total:0,controls:VecDeque::new(),stop:false,agent:if native{Some(AgentMailbox::default())}else{None} };
    drop(s);
    let _guard = Guard;
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
        else if let Some(request)=lsp.as_ref(){rt.block_on(process_probe::lsp_diagnostics(&guest,&env,request,false))}
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
                    if kind=="codex"{rt.block_on(process_probe::codex_session(&guest,&env,prompt.as_deref().unwrap_or(""),config,false))}
                    else{rt.block_on(process_probe::claude_session(&guest,&env,prompt.as_deref().unwrap_or(""),config,false))}
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
            r["binaryDigestVerified"]=json!(verified);
            r["harness"]=json!(kind);
            r["action"]=json!(action);
            r
        }
        else if let Some((token,vsix))=workbench.as_ref() {
            let installed=vsix.as_deref().map(|digest| {
                let verified=rt.block_on(process_probe::guest_file_digest(&guest,&env,"/opt/nexus/extension.vsix",digest))["passed"]==true;
                verified && !stop_requested() && rt.block_on(process_probe::local_hook_with_timeout(&guest,&env,INSTALL_VSIX,120))
            });
            let mut r=if installed.unwrap_or(true) && !stop_requested() {
                env.insert("NEXUS_WORKBENCH_TOKEN".into(),token.clone());
                rt.block_on(process_probe::local_process(&guest,&env,WORKBENCH,true))
            } else {fail("Reviewed VSIX verification or installation failed, or launch cancelled")};
            r["extensionInstalled"]=json!(installed);
            r
        } else {
            rt.block_on(process_probe::local_process(&guest,&env,input,false))
        };
    let shutdown = rt.block_on(guest.shutdown()).is_ok();
    result["shutdownClean"] = json!(shutdown);
    result["ok"] = json!(result["ok"] == true && shutdown);
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
pub unsafe extern "C" fn nexus_local_agent_send(text:*const c_char)->*mut c_char{
    ffi(||match arg(text){Some(s)=>agent_send(Some(s)),None=>fail("Missing message")})
}
#[no_mangle]
pub extern "C" fn nexus_local_agent_finish()->*mut c_char{ffi(||agent_send(None))}
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
        let config=NativeSession{retain:true,resume:if phase=="resume"{Some(handle.into())}else{None}};
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
                        assert_eq!(agent_send(Some("Second fixture".into()))["ok"],true);
                        assert_eq!(agent_send(Some("duplicate".into()))["ok"],false);
                        sent=true;
                    }else if n==2{assert_eq!(agent_send(None)["ok"],true);return true;}
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
        let r=if kind=="codex"{rt.block_on(process_probe::codex_session(&guest,&env,"First fixture",&config,true))}else{rt.block_on(process_probe::claude_session(&guest,&env,"First fixture",&config,true))};
        let p=progress();
        let clean=rt.block_on(guest.shutdown()).is_ok();
        STATE.lock().unwrap().running=false;
        let driven=driver.join().unwrap();
        let ok=driven&&r["ok"]==true&&r["turnsCompleted"]==2&&p["agentHandle"]==handle&&p["agentTurns"][0]["response"]=="First reply"&&p["agentTurns"][1]["response"]=="Second reply"&&clean;
        let evidence=json!({"ok":ok,"fixtureOnly":true,"kind":kind,"phase":phase,"turnsCompleted":r["turnsCompleted"],"shutdownClean":clean});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        assert!(ok,"conversation fixture failed: {r}");
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
    #[ignore = "requires fresh Ubuntu fakefs; synthetic LSP Content-Length peer"]
    fn real_lsp_diagnostic_fixture(){
        let path=std::env::var_os("NEXUS_LOCAL_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_LOCAL_EVIDENCE").expect("evidence");
        let phase=std::env::var("NEXUS_LSP_PHASE").expect("phase");
        let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let guest=IshInstance::boot(Path::new(&path),Some(Path::new("/"))).unwrap();
        let env=HashMap::from([("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/root".into()),("NEXUS_LSP_PHASE".into(),phase.clone())]);
        let input=parse_lsp(&json!({"argv":["/usr/bin/server"],"language":"python","extension":"py","text":"x = 1\n"}).to_string()).unwrap();
        let r=rt.block_on(process_probe::lsp_diagnostics(&guest,&env,&input,true));
        let clean=rt.block_on(guest.shutdown()).is_ok();
        let ok=if phase=="success"{r["ok"]==true&&r["diagnostics"][0]["message"]=="fixture diagnostic"&&r["diagnostics"][0]["range"]["end"]["character"]==1}else{r["ok"]==false&&r.get("diagnostics").is_none()};
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
        let stop=enqueue(Control::Stop);
        let closed=worker.join().expect("workbench owner");
        let extension_verified=vsix.as_ref().map(|_| {
            closed["extensionInstalled"]==true &&
                std::fs::read_dir(guest_path.join("root/nexus-vscode-extensions")).ok().is_some_and(|dirs|
                    dirs.filter_map(Result::ok).any(|entry| {
                        let name=entry.file_name();
                        name.to_string_lossy().starts_with("nexus.fixture-0.0.1") &&
                            entry.path().join("package.json").is_file()
                    }))
        });
        let report=json!({"ok":ready && matches!(status,Some(200|302|303|307)) && stop["ok"]==true && closed["shutdownClean"]==true && extension_verified.unwrap_or(true),
            "ready":ready,"httpStatus":status,"shutdownClean":closed["shutdownClean"],"extensionInstallVerified":extension_verified,"editorVerified":false,"lspVerified":false});
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
