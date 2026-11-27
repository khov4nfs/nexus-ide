//! Local initialize/session-new lab. A dedicated worker owns the guest and
//! Tokio runtime; no raw pointer handles or concurrent requests cross FFI.
use crate::{process_probe::{self, ACPExchange}, ATTEMPTED};
use ish_embed_host::{IshInstance, SpawnOpts};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    ffi::{c_char, CStr, CString},
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool,Ordering}, mpsc::{self, Receiver, SyncSender}, Mutex},
    thread,
    time::Duration,
};

const IDLE: Duration = Duration::from_secs(180);
const REPLY: Duration = Duration::from_secs(300);
const NEW_REQUEST: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{\"cwd\":\"/tmp\",\"mcpServers\":[]}}\n";
type Reply = SyncSender<Value>;
enum Command { New(Vec<u8>, Reply), Prompt(Option<String>, Option<String>, bool, Reply), Close(Reply) }
static ACTIVE: Mutex<Option<SyncSender<Command>>> = Mutex::new(None);
static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
struct ActiveGuard;
impl Drop for ActiveGuard { fn drop(&mut self) { WORKER_ACTIVE.store(false,Ordering::Release); } }

enum Mode { Agy(String), #[cfg(test)] Fixture }
pub(crate) enum LoginControl { None, Code(String), Cancel }
struct LoginState { running: bool, url: String, submitted: bool, cancelled: bool, code: Option<String> }
static LOGIN: Mutex<LoginState> = Mutex::new(LoginState { running:false,url:String::new(),submitted:false,cancelled:false,code:None });
struct LoginGuard;
impl LoginGuard {
    fn begin() -> Result<Self,String> {
        let mut s=LOGIN.lock().map_err(|_| "Login lock failed")?;
        *s=LoginState {running:true,url:String::new(),submitted:false,cancelled:false,code:None};
        Ok(Self)
    }
}
impl Drop for LoginGuard {
    fn drop(&mut self) {
        if let Ok(mut s)=LOGIN.lock() { s.running=false;s.url.clear();s.code=None; }
    }
}
pub(crate) fn login_publish(url:String)->Result<(),String> {
    let mut s=LOGIN.lock().map_err(|_| "Login lock failed")?;
    if !s.running || s.cancelled || s.submitted { return Err("Login not awaiting browser".into()); }
    s.url=url;
    Ok(())
}
pub(crate) fn login_take()->Result<LoginControl,String> {
    let mut s=LOGIN.lock().map_err(|_| "Login lock failed")?;
    if s.cancelled { return Ok(LoginControl::Cancel); }
    Ok(s.code.take().map(LoginControl::Code).unwrap_or(LoginControl::None))
}
fn login_progress()->Value {
    LOGIN.lock().map(|s|json!({"running":s.running,"url":s.url,"submitted":s.submitted,"cancelled":s.cancelled}))
        .unwrap_or_else(|_|failure("Login status unavailable"))
}
fn valid_login_code(code:&str)->bool {
    (8..=2048).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
}
fn submit_login_code(code:String)->Value {
    if !valid_login_code(&code) {return failure("Invalid authorization code shape");}
    let Ok(mut s)=LOGIN.lock() else {return failure("Login lock failed");};
    if !s.running || s.url.is_empty() || s.submitted || s.cancelled {return failure("Login not awaiting code");}
    s.submitted=true;s.url.clear();s.code=Some(code);
    json!({"ok":true})
}
fn cancel_login()->Value {
    let Ok(mut s)=LOGIN.lock() else {return failure("Login lock failed");};
    if !s.running || s.cancelled {return failure("No pending login");}
    s.cancelled=true;s.url.clear();s.code=None;
    json!({"ok":true})
}

// Input shape only, not authentication or validation against Google.
fn valid_key(s: &str) -> bool {
    (16..=256).contains(&s.len()) &&
        s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn failure(message: &str) -> Value {
    json!({"ok":false,"active":false,"error":message})
}

fn valid_new(bytes: &[u8]) -> bool {
    if bytes.len() > 4096 || bytes.last() != Some(&b'\n') ||
        bytes[..bytes.len()-1].contains(&b'\n') { return false; }
    let Ok(frame) = serde_json::from_slice::<Value>(bytes) else { return false; };
    frame == serde_json::from_slice::<Value>(NEW_REQUEST).expect("fixed new request")
}

fn auth_error(frame: &Value) -> bool {
    frame["error"]["code"] == -32000 && frame["error"]["message"].as_str()
        .map(|s| s.eq_ignore_ascii_case("Authentication required")).unwrap_or(false)
}

fn session_created(frame: &Value) -> bool {
    frame.get("error").is_none() && frame["result"]["sessionId"].as_str()
        .map(|s| !s.is_empty() && s.len() <= 256).unwrap_or(false)
}

const SMOKE_PROMPT: &str = "Reply with exactly NEXUS_ACP_OK. Do not use tools, read files, or perform actions.";
const SMOKE_REPLY: &str = "NEXUS_ACP_OK";

// Separate from ACTIVE, whose lock is held while a blocking command waits.
// Only counters, local control flags and filtered chat text leave this mailbox.
#[derive(Default)]
struct PromptProgress {
    running: bool,
    response_id: i64,
    chunks: usize,
    permission_denials: usize,
    tool_updates: usize,
    cancel_requested: bool,
    cancel_taken: bool,
    visible: String,
}
static PROGRESS: Mutex<PromptProgress> = Mutex::new(PromptProgress {
    running: false, response_id: 0, chunks: 0, permission_denials: 0, tool_updates: 0, cancel_requested: false, cancel_taken: false, visible: String::new(),
});

fn prompt_progress() -> Value {
    match PROGRESS.lock() {
        Ok(p) => json!({"ok":true,"running":p.running,"responseId":p.response_id,"chunkCount":p.chunks,
            "cancelRequested":p.cancel_requested,"text":p.visible,"permissionDenials":p.permission_denials,"toolUpdates":p.tool_updates}),
        Err(_) => json!({"ok":false,"running":false,"error":"Progress unavailable"}),
    }
}

fn request_cancel() -> Value {
    match PROGRESS.lock() {
        Ok(mut p) if p.running && !p.cancel_requested => {
            p.cancel_requested = true;
            json!({"ok":true,"cancelRequested":true})
        }
        _ => json!({"ok":false,"error":"No cancellable prompt or cancellation already requested"}),
    }
}

struct ProgressGuard;
impl ProgressGuard {
    fn begin(response_id: i64) -> Self {
        let mut p = PROGRESS.lock().unwrap_or_else(|e| e.into_inner());
        *p = PromptProgress { running: true, response_id, ..Default::default() };
        Self
    }
}
impl Drop for ProgressGuard {
    fn drop(&mut self) {
        PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).running = false;
    }
}

fn take_cancel(session: &str) -> Option<Vec<u8>> {
    let mut p = PROGRESS.lock().ok()?;
    if !p.running || !p.cancel_requested || p.cancel_taken { return None; }
    p.cancel_taken = true;
    let frame = json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}});
    Some(format!("{frame}\n").into_bytes())
}

const MAX_CHAT_TURNS: i64 = 20;
const CHAT_INPUT_LIMIT: usize = 1024;
const CHAT_OUTPUT_LIMIT: usize = 8192;

fn valid_chat(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= CHAT_INPUT_LIMIT &&
        text.chars().all(|c| !c.is_control() || c == '\n' || c == '\t')
}

// Literal credential protection across chunk boundaries. Never release a
// suffix that could be the start of the key. This is NOT a general DLP filter:
// transformed, encoded, or other secrets may still be returned by the agent.
fn visible_prefix<'a>(text: &'a str, key: &str) -> Result<&'a str,String> {
    if key.is_empty() || !key.is_ascii() || text.contains(key) {
        return Err("Credential output blocked".into());
    }
    for n in (1..key.len()).rev() {
        if text.ends_with(&key[..n]) {
            return Ok(&text[..text.len()-n]);
        }
    }
    Ok(text)
}

// Explicit deny-only policy. Option IDs are opaque and never shown or logged.
// Unknown methods, malformed requests, duplicate IDs and requests without a
// reject-once option fail closed. No native tools are provided.
fn permission_reply(frame: &Value, session: &str, cancelled: bool) -> Result<Value,String> {
    if frame["jsonrpc"] != "2.0" || frame["method"] != "session/request_permission" ||
        frame["params"]["sessionId"] != session || frame.get("result").is_some() ||
        frame.get("error").is_some() {
        return Err("Unsupported agent request".into());
    }
    let id = frame.get("id").filter(|id| id.as_i64().is_some() ||
        id.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256))
        .ok_or("Invalid permission request ID")?;
    let params = &frame["params"];
    if !params["toolCall"]["toolCallId"].as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256) {
        return Err("Invalid tool call ID".into());
    }
    let options = params["options"].as_array().filter(|a| !a.is_empty() && a.len() <= 16)
        .ok_or("Invalid permission options")?;
    let mut seen = std::collections::HashSet::new();
    let mut reject = None;
    for option in options {
        let option_id = option["optionId"].as_str().filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("Invalid option ID")?;
        if !seen.insert(option_id) || !option["name"].is_string() {
            return Err("Duplicate or malformed permission option".into());
        }
        match option["kind"].as_str() {
            Some("reject_once") => {
                if reject.is_some() { return Err("Ambiguous reject option".into()); }
                reject = Some(option_id);
            }
            Some("allow_once" | "allow_always" | "reject_always") => {}
            _ => return Err("Unknown permission option".into()),
        }
    }
    let outcome = if cancelled {
        json!({"outcome":"cancelled"})
    } else {
        json!({"outcome":"selected","optionId":reject.ok_or("No reject-once option")?})
    };
    Ok(json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}}))
}

const SNAPSHOT_PATH:&str="/nexus-selected.txt";
fn valid_snapshot(s:&str)->bool {
    !s.trim().is_empty() && s.len()<=768 && s.chars().all(|c| !c.is_control() || c=='\n' || c=='\t')
}
// Never opens a filesystem path. Only serves consented bytes or the validated
// staged replacement for the current turn and one virtual path.
fn read_snapshot_reply(frame:&Value,session:&str,snapshot:Option<&str>)->Result<Value,String> {
    if frame["jsonrpc"]!="2.0" || frame["method"]!="fs/read_text_file" ||
        frame.get("result").is_some() || frame.get("error").is_some() {return Err("Invalid read request".into());}
    let id=frame.get("id").filter(|v|v.as_i64().is_some() || v.as_str().is_some_and(|s|!s.is_empty()&&s.len()<=256)).ok_or("Invalid read ID")?;
    let deny=||json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"No matching snapshot grant"}});
    let params=&frame["params"];
    if params["sessionId"]!=session || params["path"]!=SNAPSHOT_PATH {return Ok(deny());}
    let Some(text)=snapshot.filter(|s|s.len()<=4096 && s.chars().all(|c|!c.is_control() || c=='\n' || c=='\t')) else {return Ok(deny());};
    let line=match params.get("line") {None|Some(Value::Null)=>1,Some(v)=>match v.as_u64(){Some(n) if n>=1 && n<=4097=>n as usize,_=>return Ok(deny())}};
    let limit=match params.get("limit") {None|Some(Value::Null)=>4097,Some(v)=>match v.as_u64(){Some(n) if n>=1 && n<=4097=>n as usize,_=>return Ok(deny())}};
    let content=if line==1 && params.get("limit").is_none_or(Value::is_null) {text.to_owned()} else {
        text.split('\n').skip(line-1).take(limit).collect::<Vec<_>>().join("\n")
    };
    Ok(json!({"jsonrpc":"2.0","id":id,"result":{"content":content}}))
}
// ACP write is virtual: stage a replacement, never touch a native path.
fn stage_write_reply(frame:&Value,session:&str,allowed:bool,key:&str,staged:&mut Option<String>)->Result<Value,String> {
    if frame["jsonrpc"]!="2.0" || frame["method"]!="fs/write_text_file" ||
        frame.get("result").is_some() || frame.get("error").is_some() {return Err("Invalid write request".into());}
    let id=frame.get("id").filter(|v|v.as_i64().is_some() || v.as_str().is_some_and(|s|!s.is_empty()&&s.len()<=256)).ok_or("Invalid write ID")?;
    let deny=||json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"No matching staged write grant"}});
    let params=&frame["params"];
    if !allowed || staged.is_some() || params["sessionId"]!=session || params["path"]!=SNAPSHOT_PATH {return Ok(deny());}
    let Some(text)=params["content"].as_str().filter(|s|s.len()<=4096 && !key.is_empty() && !s.contains(key) &&
        s.chars().all(|c|!c.is_control() || c=='\n' || c=='\t')) else {return Ok(deny());};
    *staged=Some(text.to_owned());
    Ok(json!({"jsonrpc":"2.0","id":id,"result":{}}))
}
#[derive(Default)]
struct PromptEvidence {
    text: String,
    chunks: usize,
    message_id: Option<String>,
    tool_updates: usize,
}
impl PromptEvidence {
    fn update(&mut self, frame: &Value, session: &str) -> Result<(),String> {
        if frame["method"] != "session/update" || frame["params"]["sessionId"] != session {
            return Err("Unexpected prompt notification".into());
        }
        let u = &frame["params"]["update"];
        match u["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                let content = &u["content"];
                let text = content["text"].as_str().ok_or("Missing text chunk")?;
                if content["type"] != "text" || self.chunks >= 128 ||
                    self.text.len() + text.len() > CHAT_OUTPUT_LIMIT { return Err("Unsupported or oversized text".into()); }
                if let Some(id) = u.get("messageId") {
                    let id = id.as_str().filter(|s| !s.is_empty() && s.len() <= 256)
                        .ok_or("Invalid message id")?;
                    if self.message_id.as_deref().is_some_and(|old| old != id) {
                        return Err("Multiple agent messages not supported".into());
                    }
                    self.message_id = Some(id.to_owned());
                }
                self.text.push_str(text);
                self.chunks += 1;
                Ok(())
            }
            Some("tool_call" | "tool_call_update") => {
                // Observation only. Do not accept execution/completion or raw
                // tool content as proof that a denied action did not happen.
                if !u["toolCallId"].as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256) ||
                    self.tool_updates >= 32 {
                    return Err("Invalid or excessive tool updates".into());
                }
                match u["status"].as_str() {
                    Some("pending" | "failed") => {}
                    None if u.get("status").is_none() && u["sessionUpdate"] == "tool_call" => {}
                    _ => return Err("Tool execution or unsupported status reported".into()),
                }
                self.tool_updates += 1;
                Ok(())
            }
            Some("agent_thought_chunk" | "user_message_chunk" | "available_commands_update" |
                 "current_mode_update" | "config_option_update" | "session_info_update" | "usage_update") => Ok(()),
            // Observing a tool notification is NOT a preventive sandbox.
            _ => Err("Unsupported session update; stopping probe".into()),
        }
    }
    fn complete(&self, frame: &Value) -> bool {
        frame["jsonrpc"] == "2.0" && frame["id"] == 3 &&
            frame.get("error").is_none() && frame["result"]["stopReason"] == "end_turn" &&
            self.chunks > 0 && self.text.len() <= 1024 && self.text.trim() == SMOKE_REPLY
    }
}

fn worker(path: PathBuf, mode: Mode, request: Vec<u8>, key: Option<String>, google: bool, commands: Receiver<Command>, ready: Reply) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt, Err(_) => { let _ = ready.send(failure("Runtime creation failed")); return; }
    };
    let guest = match IshInstance::boot(&path, Some(Path::new("/"))) {
        Ok(g) => g, Err(_) => { let _ = ready.send(failure("Guest boot failed; restart app")); return; }
    };
    let env = HashMap::from([("PATH".to_owned(),"/usr/bin:/bin:/usr/sbin:/sbin".to_owned()),
        ("HOME".to_owned(),"/root".to_owned()),("LANG".to_owned(),"C".to_owned()),
        ("AGY_CLI_DISABLE_AUTO_UPDATE".to_owned(),"true".to_owned())]);
    let libc = rt.block_on(process_probe::glibc_compat(&guest, &env));
    if libc["passed"] != true {
        let _ = rt.block_on(guest.shutdown());
        let _ = ready.send(failure("Ubuntu compatibility failed")); return;
    }
    let mut opts = match &mode {
        Mode::Agy(sha) => {
            let digest = rt.block_on(process_probe::agy_binary_digest(&guest, &env, sha));
            if digest["passed"] != true {
                let _ = rt.block_on(guest.shutdown());
                let _ = ready.send(failure("Guest AGY digest mismatch")); return;
            }
            SpawnOpts::cmd(["/opt/nexus/agy", "acp", "--disable-telemetry"])
        }
        #[cfg(test)]
        Mode::Fixture => SpawnOpts::cmd(["/bin/sh", "-c", include_str!("../fixtures/acp-transport.sh")]),
    };
    opts.cwd = Some(PathBuf::from("/tmp"));
    let keyed = key.is_some();
    let mut credential_filter = key.clone();
    if google {
        let fixture = match &mode { Mode::Agy(_) => false, #[cfg(test)] Mode::Fixture => true };
        let login = match LoginGuard::begin() {
            Ok(guard) => {
                let result=rt.block_on(process_probe::google_login(&guest,&env,fixture));
                drop(guard);
                result
            }
            Err(e)=>Err(e),
        };
        match login {
            Ok(code)=>credential_filter=Some(code),
            Err(_)=>{
                let shutdown=rt.block_on(guest.shutdown()).is_ok();
                let _=ready.send(json!({"ok":false,"active":false,"shutdownClean":shutdown,"error":"Google terminal login failed or cancelled; restart app"}));
                return;
            }
        }
    }
    if keyed && !rt.block_on(process_probe::prepare_gemini_provider(&guest, &env)) {
        let _ = rt.block_on(guest.shutdown());
        let _ = ready.send(failure("Gemini provider preparation failed")); return;
    }
    let mut agent_env = env;
    if let Some(key) = key { agent_env.insert("GEMINI_API_KEY".into(), key); }
    #[cfg(test)]
    if google && matches!(&mode,Mode::Fixture) {agent_env.insert("NEXUS_FIXTURE_GOOGLE".into(),"1".into());}
    #[cfg(test)]
    if matches!(&mode, Mode::Fixture) && std::env::var("NEXUS_ACP_TEST_CANCEL").as_deref() == Ok("1") {
        agent_env.insert("NEXUS_FIXTURE_CANCEL".into(), "1".into());
    }
    #[cfg(test)]
    if matches!(&mode, Mode::Fixture) && std::env::var("NEXUS_ACP_TEST_CHAT").as_deref() == Ok("1") {
        agent_env.insert("NEXUS_FIXTURE_CHAT".into(), "1".into());
    }
    #[cfg(test)]
    if matches!(&mode,Mode::Fixture) && std::env::var("NEXUS_ACP_TEST_SNAPSHOT").as_deref()==Ok("1") {agent_env.insert("NEXUS_FIXTURE_SNAPSHOT".into(),"1".into());}
    opts.envp = Some(agent_env.into_iter().collect());
    opts.pipe_stdin = true;
    let session = match guest.spawn(opts) {
        Ok(s) => s, Err(_) => {
            let _ = rt.block_on(guest.shutdown());
            let _ = ready.send(failure("Agent spawn failed")); return;
        }
    };
    let mut exchange = ACPExchange::new(session);
    let edit_capability=process_probe::edit_initialize_request(&request);
    let snapshot_capability=edit_capability || process_probe::snapshot_initialize_request(&request);
    let init = rt.block_on(exchange.request(&request, 1));
    let response = match init {
        Ok(frame) => match &mode {
            Mode::Agy(_) => process_probe::initialize_projection(&frame).map(|mut response| {
                // In credential mode never export agent-supplied free text.
                if (keyed || google) && response.get("result").is_some() {
                    response["result"]["agentInfo"] = json!({"name":"Antigravity","version":"withheld"});
                }
                response
            }),
            #[cfg(test)]
            Mode::Fixture => {
                let expected = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,
                    "agentInfo":{"name":"Nexus Transport Fixture","version":"test"}}});
                if frame == expected { Some(frame) } else { None }
            }
        },
        Err(_) => None,
    };
    let init_ok = response.as_ref().is_some_and(|r| r.get("result").is_some());
    if !init_ok {
        let auth = response.as_ref().is_some_and(auth_error);
        let closed = rt.block_on(exchange.close());
        let shutdown = rt.block_on(guest.shutdown()).is_ok();
        let _ = ready.send(json!({"ok":false,"active":false,"authenticationRequired":auth,
            "transportClosed":closed,"shutdownClean":shutdown,"error":"Initialize rejected"}));
        return;
    }
    // The worker retains exclusive ownership across the native FFI calls.
    // If the caller disappears, a bounded idle wait closes the process.
    WORKER_ACTIVE.store(true,Ordering::Release);
    let _active_guard = ActiveGuard;
    if ready.send(json!({"ok":true,"active":true,"response":response,"credentialMode":if google {"google-terminal-login"} else if keyed {"gemini-api-key"} else {"none"}})).is_err() {
        let _ = rt.block_on(exchange.close());
        let _ = rt.block_on(guest.shutdown());
        return;
    }
    let mut session_id: Option<String> = None;
    let mut turns: i64 = 0;
    let mut smoke_used = false;
    let final_reply = loop {
        match commands.recv_timeout(IDLE) {
            Ok(Command::New(bytes, reply)) => {
                if session_id.is_some() || !valid_new(&bytes) {
                    let _ = reply.send(json!({"ok":false,"active":true,"error":"Only one fixed session/new is allowed"}));
                    continue;
                }
                let result = rt.block_on(exchange.request(&bytes, 2));
                match result {
                    Ok(frame) if session_created(&frame) => {
                        #[cfg(test)]
                        if matches!(&mode, Mode::Fixture) &&
                            !frame["result"]["sessionId"].as_str().unwrap_or("").starts_with("fixture-") {
                            let _ = reply.send(failure("Fixture session ID mismatch"));
                            break None;
                        }
                        session_id = frame["result"]["sessionId"].as_str().map(str::to_owned);
                        if reply.send(json!({"ok":true,"active":true,"sessionCreated":true,"responseId":2})).is_err() {
                            break None;
                        }
                    }
                    other => {
                        let auth = other.as_ref().ok().is_some_and(auth_error);
                        break Some((reply, json!({"ok":false,"active":false,"authenticationRequired":auth,"error":"Session creation rejected"})));
                    }
                }
            }
            Ok(Command::Prompt(chat, snapshot, edit, reply)) => {
                let Some(id) = session_id.as_deref() else {
                    let _ = reply.send(json!({"ok":false,"active":true,"error":"Create a session first"}));
                    continue;
                };
                if !(keyed || google) || smoke_used || turns >= MAX_CHAT_TURNS || (chat.is_none() && turns != 0) {
                    let _ = reply.send(json!({"ok":false,"active":true,"error":"Chat limit reached or smoke test is exclusive"}));
                    continue;
                }
                let Some(key) = credential_filter.as_deref() else {
                    let _ = reply.send(json!({"ok":false,"active":true,"error":"Credential unavailable"}));
                    continue;
                };
                if chat.as_ref().is_some_and(|text| !valid_chat(text) || text.contains(key)) {
                    let _ = reply.send(json!({"ok":false,"active":true,"error":"Invalid chat input or credential detected"}));
                    continue;
                }
                if (edit && (!edit_capability || snapshot.is_none())) || snapshot.as_ref().is_some_and(|s| !snapshot_capability || chat.is_none() ||
                    !valid_snapshot(s) || s.contains(key)) {
                    let _=reply.send(json!({"ok":false,"active":true,"error":"Snapshot grant unavailable or invalid"}));
                    continue;
                }
                let is_chat = chat.is_some();
                let text = chat.as_deref().unwrap_or(SMOKE_PROMPT);
                turns += 1;
                smoke_used = !is_chat;
                let response_id = turns + 2;
                let request = json!({"jsonrpc":"2.0","id":response_id,"method":"session/prompt",
                    "params":{"sessionId":id,"prompt":[{"type":"text","text":text}]}});
                let wire = format!("{request}\n");
                let mut evidence = PromptEvidence::default();
                let progress = ProgressGuard::begin(response_id);
                let mut cancel_taken = false;
                let mut permission_ids = std::collections::HashSet::new();
                let mut snapshot_reads=0usize;
                let mut staged:Option<String>=None;
                let result = rt.block_on(exchange.request_controlled(wire.as_bytes(), response_id,
                    |frame| {
                        evidence.update(frame, id)?;
                        // Check the assembled text, not each chunk independently.
                        let safe = visible_prefix(&evidence.text, key)?;
                        if evidence.text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                            return Err("Unsupported output control character".into());
                        }
                        if !is_chat && evidence.text.len() > 1024 {
                            return Err("Smoke output too large".into());
                        }
                        if let Ok(mut p) = PROGRESS.lock() {
                            p.chunks = evidence.chunks;
                            p.tool_updates = evidence.tool_updates;
                            if is_chat { p.visible = safe.to_owned(); }
                        }
                        Ok(())
                    },
                    || {
                        let wire = take_cancel(id);
                        cancel_taken |= wire.is_some();
                        wire
                    },
                    |frame| {
                        if permission_ids.len() >= 8 {
                            return Err("Permission request limit exceeded".into());
                        }
                        let mut p = PROGRESS.lock().map_err(|_| "Permission status unavailable")?;
                        let response = if frame["method"]=="fs/read_text_file" && snapshot_capability {
                            let granted=if p.cancel_requested || snapshot_reads>=4 {None} else {staged.as_deref().or(snapshot.as_deref())};
                            let reply=read_snapshot_reply(frame,id,granted)?;
                            if reply.get("result").is_some() {snapshot_reads+=1;}
                            reply
                        } else if frame["method"]=="fs/write_text_file" {
                            stage_write_reply(frame,id,edit && !p.cancel_requested && staged.is_none(),key,&mut staged)?
                        } else {permission_reply(frame, id, p.cancel_requested)?};
                        if !permission_ids.insert(response["id"].to_string()) {
                            return Err("Duplicate permission request ID".into());
                        }
                        if response["result"]["outcome"]["outcome"] == "selected" {
                            p.permission_denials += 1;
                        }
                        Ok(response)
                    }));
                drop(progress);
                let cancel_requested = PROGRESS.lock().map(|p|p.cancel_requested).unwrap_or(true);
                let cancelled = cancel_taken && result.as_ref().is_ok_and(|frame|
                    frame.get("error").is_none() && frame["result"]["stopReason"] == "cancelled");
                if cancelled {
                    // Confirm only the response to the original prompt. No raw text.
                    if reply.send(json!({"ok":true,"active":true,"cancelled":true,
                        "responseId":response_id,"stopReason":"cancelled","chunkCount":evidence.chunks})).is_err() {
                        break None;
                    }
                    continue;
                }
                let chat_success = is_chat && result.as_ref().is_ok_and(|frame|
                    frame.get("error").is_none() && frame["result"]["stopReason"] == "end_turn" &&
                    (staged.is_some() || (evidence.chunks > 0 && visible_prefix(&evidence.text, key)
                        .is_ok_and(|s| !s.trim().is_empty()))));
                if chat_success {
                    if reply.send(json!({"ok":true,"active":true,"chatCompleted":true,
                        "responseId":response_id,"chunkCount":evidence.chunks,"stopReason":"end_turn","snapshotReads":snapshot_reads,"stagedReplacement":if !cancel_requested { staged.take() } else { None }})).is_err() {
                        break None;
                    }
                    continue;
                }
                let success = !is_chat && result.as_ref().is_ok_and(|frame| evidence.complete(frame));
                if success {
                    // Export a constant marker only, never model text, IDs or errors.
                    if reply.send(json!({"ok":true,"active":true,"promptVerified":true,
                        "responseId":response_id,"chunkCount":evidence.chunks,"stopReason":"end_turn",
                        "marker":SMOKE_REPLY})).is_err() { break None; }
                } else {
                    if let Ok(mut p) = PROGRESS.lock() { p.visible.clear(); }
                    let auth = result.as_ref().ok().is_some_and(auth_error);
                    break Some((reply,json!({"ok":false,"active":false,"authenticationRequired":auth,
                        "error":"Prompt rejected, blocked or incomplete"})));
                }
            }
            Ok(Command::Close(reply)) => break Some((reply, json!({"ok":true,"active":false}))),
            Err(_) => break None,
        }
    };
    let closed = rt.block_on(exchange.close());
    let shutdown = rt.block_on(guest.shutdown()).is_ok();
    if let Some((reply, mut result)) = final_reply {
        result["transportClosed"] = json!(closed);
        result["shutdownClean"] = json!(shutdown);
        result["ok"] = json!(result["ok"] == true && closed && shutdown);
        let _ = reply.send(result);
    }
}

fn start(path: PathBuf, mode: Mode, request: Vec<u8>, key: Option<String>, google: bool) -> Value {
    if google && key.is_some() { return failure("Choose one authentication method"); }
    if key.as_deref().is_some_and(|s| !valid_key(s)) { return failure("Invalid credential shape"); }
    if !process_probe::valid_initialize_request(&request) && !process_probe::snapshot_initialize_request(&request) && !process_probe::edit_initialize_request(&request) {
        return failure("Invalid initialize request");
    }
    if let Mode::Agy(sha) = &mode {
        if !process_probe::agy_digest_ok(sha) { return failure("Invalid AGY SHA-256"); }
    }
    if !path.is_dir() || !path.parent().is_some_and(|p| p.join("meta.db").is_file()) {
        return failure("Missing prepared fakefs");
    }
    let Ok(mut slot) = ACTIVE.lock() else { return failure("Connection lock failed"); };
    if slot.is_some() || ATTEMPTED.swap(true, Ordering::AcqRel) {
        return failure("Restart app before another boot");
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let (reply, wait) = mpsc::sync_channel(1);
    if thread::Builder::new().name("nexus-acp-owner".into())
        .spawn(move || worker(path, mode, request, key, google, rx, reply)).is_err() {
        return failure("Worker creation failed");
    }
    match wait.recv_timeout(REPLY) {
        Ok(report) => {
            if report["active"] == true { *slot = Some(tx); }
            report
        }
        Err(_) => failure("Startup reply timed out; restart app"),
    }
}

enum Action { New(Vec<u8>), Prompt, Chat(String), Snapshot(String,String), Edit(String,String), Close }

fn command(action: Action) -> Value {
    let Ok(mut slot) = ACTIVE.lock() else { return failure("Connection lock failed"); };
    let Some(tx) = slot.as_ref() else { return failure("No active connection"); };
    let (reply, wait) = mpsc::sync_channel(1);
    let message = match action { Action::New(b) => Command::New(b, reply), Action::Prompt => Command::Prompt(None, None, false, reply), Action::Chat(text) => Command::Prompt(Some(text), None, false, reply), Action::Snapshot(text,content) => Command::Prompt(Some(text),Some(content),false,reply), Action::Edit(text,content) => Command::Prompt(Some(text),Some(content),true,reply), Action::Close => Command::Close(reply) };
    if tx.send(message).is_err() {
        *slot = None; return failure("Connection stopped or idle timeout");
    }
    match wait.recv_timeout(REPLY) {
        Ok(report) => {
            if report["active"] != true { *slot = None; }
            report
        }
        Err(_) => { *slot = None; failure("Connection reply timed out; restart app") }
    }
}

fn ffi_result(f: impl FnOnce() -> Value + std::panic::UnwindSafe) -> *mut c_char {
    let report = std::panic::catch_unwind(f).unwrap_or_else(|_| failure("Runtime panic; restart app"));
    CString::new(report.to_string()).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

unsafe fn read_arg(p: *const c_char) -> Option<String> {
    if p.is_null() { return None; }
    CStr::from_ptr(p).to_str().ok().map(str::to_owned)
}

/// # Safety
/// All three pointers must reference valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_start(path: *const c_char, digest: *const c_char, request: *const c_char) -> *mut c_char {
    ffi_result(|| {
        match (read_arg(path), read_arg(digest), read_arg(request)) {
            (Some(path), Some(sha), Some(req)) => start(PathBuf::from(path), Mode::Agy(sha), req.into_bytes(), None, false),
            _ => failure("Missing connection arguments"),
        }
    })
}

/// # Safety
/// All pointers must reference valid NUL-terminated UTF-8 strings.
/// Only call after explicit consent. Key is passed in guest process environment.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_start_gemini(path: *const c_char, digest: *const c_char, request: *const c_char, key: *const c_char) -> *mut c_char {
    ffi_result(|| match (read_arg(path), read_arg(digest), read_arg(request), read_arg(key)) {
        (Some(path), Some(sha), Some(req), Some(key)) => start(PathBuf::from(path), Mode::Agy(sha), req.into_bytes(), Some(key), false),
        _ => failure("Missing connection arguments"),
    })
}

/// # Safety
/// Pointers reference NUL-terminated UTF-8. Explicit consent required.
/// Login process and ACP share one guest; never return raw login output.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_start_google(path:*const c_char,digest:*const c_char,request:*const c_char)->*mut c_char {
    ffi_result(||match (read_arg(path),read_arg(digest),read_arg(request)) {
        (Some(p),Some(sha),Some(req))=>start(PathBuf::from(p),Mode::Agy(sha),req.into_bytes(),None,true),
        _=>failure("Missing connection arguments"),
    })
}
#[no_mangle]
pub extern "C" fn nexus_google_login_progress()->*mut c_char {ffi_result(login_progress)}
/// # Safety
/// Code pointer must reference a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn nexus_google_login_code(code:*const c_char)->*mut c_char {
    ffi_result(||read_arg(code).map(submit_login_code).unwrap_or_else(||failure("Missing code")))
}
#[no_mangle]
pub extern "C" fn nexus_google_login_cancel()->*mut c_char {ffi_result(cancel_login)}
#[no_mangle]
pub extern "C" fn nexus_acp_connection_status()->*mut c_char {
    ffi_result(||json!({"active":WORKER_ACTIVE.load(Ordering::Acquire)}))
}

/// # Safety
/// request must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_new_session(request: *const c_char) -> *mut c_char {
    ffi_result(|| match read_arg(request) {
        Some(req) if valid_new(req.as_bytes()) => command(Action::New(req.into_bytes())),
        _ => failure("Invalid session/new request"),
    })
}

// Fixed prompt only; caller must obtain explicit model-network consent.
#[no_mangle]
pub extern "C" fn nexus_acp_prompt_probe() -> *mut c_char {
    ffi_result(|| command(Action::Prompt))
}

/// # Safety
/// text must be valid NUL-terminated UTF-8. User must consent to model inference.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_chat(text: *const c_char) -> *mut c_char {
    ffi_result(|| match read_arg(text) {
        Some(text) if valid_chat(&text) => command(Action::Chat(text)),
        _ => json!({"ok":false,"active":true,"error":"Invalid chat input"}),
    })
}

/// # Safety
/// Both arguments are NUL-terminated UTF-8; caller must obtain explicit consent
/// to share the selected snapshot with the agent and model on this turn only.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_chat_snapshot(text:*const c_char,content:*const c_char)->*mut c_char {
    ffi_result(||match (read_arg(text),read_arg(content)) {
        (Some(t),Some(c)) if valid_chat(&t)&&valid_snapshot(&c)=>command(Action::Snapshot(t,c)),
        _=>failure("Invalid snapshot request"),
    })
}

// These two functions do not acquire the blocking command lock.
#[no_mangle]
pub extern "C" fn nexus_acp_prompt_progress() -> *mut c_char {
    ffi_result(prompt_progress)
}

#[no_mangle]
pub extern "C" fn nexus_acp_cancel_prompt() -> *mut c_char {
    ffi_result(request_cancel)
}

#[no_mangle]
pub extern "C" fn nexus_acp_close() -> *mut c_char {
    ffi_result(|| command(Action::Close))
}

#[no_mangle]
pub unsafe extern "C" fn nexus_acp_chat_edit(text:*const c_char,content:*const c_char)->*mut c_char {
    ffi_result(||match (read_arg(text),read_arg(content)) {
        (Some(t),Some(c)) if valid_chat(&t)&&valid_snapshot(&c)=>command(Action::Edit(t,c)),
        _=>failure("Invalid staged edit request"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_write_scope_and_limits() {
        let mut staged=None;
        let mut f=json!({"jsonrpc":"2.0","id":77,"method":"fs/write_text_file","params":{"sessionId":"s","path":SNAPSHOT_PATH,"content":"new"}});
        assert!(stage_write_reply(&f,"s",false,"SECRET",&mut staged).unwrap().get("error").is_some());
        assert!(stage_write_reply(&f,"other",true,"SECRET",&mut staged).unwrap().get("error").is_some());
        f["params"]["path"]=json!("/etc/passwd");
        assert!(stage_write_reply(&f,"s",true,"SECRET",&mut staged).unwrap().get("error").is_some());
        f["params"]["path"]=json!(SNAPSHOT_PATH);
        f["params"]["content"]=json!("SECRET");
        assert!(stage_write_reply(&f,"s",true,"SECRET",&mut staged).unwrap().get("error").is_some());
        f["params"]["content"]=json!("x".repeat(4097));
        assert!(stage_write_reply(&f,"s",true,"SECRET",&mut staged).unwrap().get("error").is_some());
        f["params"]["content"]=json!("new");
        assert!(stage_write_reply(&f,"s",true,"SECRET",&mut staged).unwrap().get("result").is_some());
        assert_eq!(staged.as_deref(),Some("new"));
        assert!(stage_write_reply(&f,"s",true,"SECRET",&mut staged).unwrap().get("error").is_some());
    }
    #[test]
    fn virtual_read_scope_and_line_bounds() {
        let mut f=json!({"jsonrpc":"2.0","id":77,"method":"fs/read_text_file","params":{"sessionId":"s","path":SNAPSHOT_PATH,"line":2,"limit":1}});
        assert_eq!(read_snapshot_reply(&f,"s",Some("one\nhai\nthree")).unwrap()["result"]["content"],"hai");
        assert!(read_snapshot_reply(&f,"s",None).unwrap().get("error").is_some());
        assert!(read_snapshot_reply(&f,"other",Some("one")).unwrap().get("error").is_some());
        f["params"]["path"]=json!("/etc/passwd");
        assert!(read_snapshot_reply(&f,"s",Some("one")).unwrap().get("error").is_some());
        f["params"]["path"]=json!(SNAPSHOT_PATH);
        f["params"]["line"]=json!(0);
        assert!(read_snapshot_reply(&f,"s",Some("one")).unwrap().get("error").is_some());
        f["method"]=json!("fs/write_text_file");
        assert!(read_snapshot_reply(&f,"s",Some("one")).is_err());
    }
    #[test]
    fn restricted_request_and_session_shape() {
        assert!(valid_key("NEXUS_TEST_ONLY_NOT_A_REAL_KEY"));
        assert!(!valid_key(""));
        assert!(!valid_key("not a valid secret value"));
        assert!(!valid_key(&"a".repeat(257)));
        assert!(valid_new(NEW_REQUEST));
        assert!(!valid_new(b""));
        assert!(!valid_new(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/prompt\"}\n"));
        assert!(!session_created(&json!({"result":{"sessionId":""}})));
        assert!(session_created(&json!({"result":{"sessionId":"fixture-1"}})));
    }

    #[test]
    fn prompt_chunks_and_stop_reason_are_strict() {
        let chunk = |session: &str, text: &str| json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":session,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}}});
        let mut e = PromptEvidence::default();
        assert!(e.update(&chunk("wrong","NEXUS_"),"s").is_err());
        e.update(&chunk("s","NEXUS_"),"s").unwrap();
        e.update(&chunk("s","ACP_OK"),"s").unwrap();
        assert!(e.complete(&json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}})));
        assert!(!e.complete(&json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}})));
        assert!(e.update(&json!({"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call"}}}),"s").is_err());
        assert!(e.update(&chunk("s",&"a".repeat(CHAT_OUTPUT_LIMIT+1)),"s").is_err());
    }

    #[test]
    fn literal_credential_filter_holds_split_prefix() {
        let key = "NEXUS_TEST_ONLY_NOT_A_REAL_KEY";
        assert_eq!(visible_prefix("Tiếng Việt NEXUS_TEST_", key).unwrap(), "Tiếng Việt ");
        assert!(visible_prefix("Tiếng Việt NEXUS_TEST_ONLY_NOT_A_REAL_KEY", key).is_err());
        assert_eq!(visible_prefix("Tiếng Việt — ok", key).unwrap(), "Tiếng Việt — ok");
        assert_eq!(visible_prefix("NEXUS", key).unwrap(), "");
        assert!(valid_chat("Xin chào\n"));
        assert!(!valid_chat(""));
        assert!(!valid_chat(" \n"));
        assert!(!valid_chat("a\0b"));
        assert!(!valid_chat(&"é".repeat(513)));
    }

    #[test]
    fn permission_policy_never_grants() {
        let frame = json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission",
            "params":{"sessionId":"s","toolCall":{"toolCallId":"tool-1"},"options":[
                {"optionId":"reject-looking","name":"Reject","kind":"allow_once"},
                {"optionId":"opaque-deny","name":"Deny","kind":"reject_once"}]}});
        let reply = permission_reply(&frame,"s",false).unwrap();
        assert_eq!(reply, json!({"jsonrpc":"2.0","id":"permission-1","result":{"outcome":{"outcome":"selected","optionId":"opaque-deny"}}}));
        assert_eq!(permission_reply(&frame,"s",true).unwrap()["result"]["outcome"],json!({"outcome":"cancelled"}));
        assert!(permission_reply(&frame,"wrong",false).is_err());
        let mut invalid = frame.clone();
        invalid["params"]["options"][1]["kind"] = json!("allow_always");
        assert!(permission_reply(&invalid,"s",false).is_err());
        invalid = frame.clone();
        invalid["params"]["options"][0]["optionId"] = json!("opaque-deny");
        assert!(permission_reply(&invalid,"s",false).is_err());
        invalid = frame.clone();
        invalid["method"] = json!("fs/write_text_file");
        assert!(permission_reply(&invalid,"s",false).is_err());
        let mut evidence = PromptEvidence::default();
        let update = |status: &str| json!({"method":"session/update","params":{"sessionId":"s",
            "update":{"sessionUpdate":"tool_call","toolCallId":"t","status":status}}});
        assert!(evidence.update(&update("pending"),"s").is_ok());
        assert!(evidence.update(&update("in_progress"),"s").is_err());
        assert!(evidence.update(&update("completed"),"s").is_err());
        assert!(evidence.update(&update("failed"),"s").is_ok());
        assert_eq!(evidence.tool_updates,2);
    }

    #[test]
    fn google_code_validation() {
        assert!(valid_login_code("4/fixture-code_123"));
        assert!(!valid_login_code("abc"));
        assert!(!valid_login_code("4/code\nextra"));
        assert!(!valid_login_code(&"a".repeat(2049)));
    }

    #[test]
    #[ignore = "fresh Ubuntu fakefs; synthetic ACP snapshot peer only"]
    fn real_snapshot_read_transport() {
        let path=std::env::var_os("NEXUS_ACP_ROOTFS").expect("rootfs");
        let file=std::env::var_os("NEXUS_ACP_EVIDENCE").expect("evidence");
        let mut req:Value=serde_json::from_slice(process_probe::AGY_INITIALIZE_REQUEST).unwrap();
        req["params"]["clientCapabilities"]=json!({"fs":{"readTextFile":true,"writeTextFile":true}});
        let init=start(PathBuf::from(path),Mode::Fixture,format!("{req}\n").into_bytes(),Some("NEXUS_TEST_ONLY_NOT_A_REAL_KEY".into()),false);
        let new=command(Action::New(NEW_REQUEST.to_vec()));
        let prompt=command(Action::Snapshot("Read /nexus-selected.txt".into(),"one\nforty-two\nthree".into()));
        let response=prompt_progress()["text"]=="NEXUS_SNAPSHOT_OK";
        let edit=command(Action::Edit("Stage replacement".into(),"one\nforty-two\nthree".into()));
        let write_verified=edit["chatCompleted"]==true && edit["stagedReplacement"]=="updated" && edit["snapshotReads"]==1;
        let close=command(Action::Close);
        let evidence=json!({"ok":init["ok"]==true && new["ok"]==true && prompt["chatCompleted"]==true &&
            prompt["snapshotReads"]==1 && response && write_verified && close["shutdownClean"]==true,
            "fixtureOnly":true,"scopeVerified":response,"stagedWriteVerified":write_verified,"snapshotReads":prompt["snapshotReads"],"shutdownClean":close["shutdownClean"]});
        std::fs::write(file,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        println!("{evidence}");
        assert_eq!(evidence["ok"],true);
    }
    #[test]
    #[ignore = "requires separate fresh Ubuntu fakefs; synthetic Google login only"]
    fn real_google_login_transport() {
        let path=std::env::var_os("NEXUS_GOOGLE_ROOTFS").expect("rootfs");
        let report=std::env::var_os("NEXUS_GOOGLE_EVIDENCE").expect("evidence");
        let worker=thread::spawn(move||start(PathBuf::from(path),Mode::Fixture,process_probe::AGY_INITIALIZE_REQUEST.to_vec(),None,true));
        let deadline=std::time::Instant::now()+Duration::from_secs(15);
        let offered=loop {
            if login_progress()["url"].as_str().is_some_and(|s| s.starts_with("https://accounts.google.com/o/oauth2/")) {break true;}
            if worker.is_finished() || std::time::Instant::now()>=deadline {break false;}
            thread::sleep(Duration::from_millis(20));
        };
        let submitted=submit_login_code("NEXUS_FIXTURE_AUTH_CODE".into());
        let duplicate=submit_login_code("NEXUS_FIXTURE_AUTH_CODE".into());
        if !offered || submitted["ok"]!=true {let _=cancel_login();}
        let first=worker.join().expect("worker");
        let new=command(Action::New(NEW_REQUEST.to_vec()));
        let prompt=command(Action::Prompt);
        let close=command(Action::Close);
        let evidence=json!({"ok":offered && submitted["ok"]==true && duplicate["ok"]==false &&
            first["ok"]==true && first["credentialMode"]=="google-terminal-login" &&
            new["sessionCreated"]==true && prompt["promptVerified"]==true && close["shutdownClean"]==true &&
            login_progress()["running"]==false && login_progress()["url"]=="",
            "fixtureOnly":true,"browserOffer":offered,"sameGuestSession":new["sessionCreated"],"promptVerified":prompt["promptVerified"],"shutdownClean":close["shutdownClean"]});
        std::fs::write(report,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        println!("{evidence}");
        assert_eq!(evidence["ok"],true);
    }

    #[test]
    #[ignore = "requires a separate fresh Ubuntu fakefs"]
    fn real_persistent_acp_transport() {
        let path = std::env::var_os("NEXUS_ACP_ROOTFS").expect("set NEXUS_ACP_ROOTFS");
        let evidence = std::env::var_os("NEXUS_ACP_EVIDENCE").expect("set NEXUS_ACP_EVIDENCE");
        let first = start(PathBuf::from(path), Mode::Fixture, process_probe::AGY_INITIALIZE_REQUEST.to_vec(), Some("NEXUS_TEST_ONLY_NOT_A_REAL_KEY".into()), false);
        let before_session = command(Action::Prompt);
        let second = command(Action::New(NEW_REQUEST.to_vec()));
        let duplicate = command(Action::New(NEW_REQUEST.to_vec()));
        let cancel_mode = std::env::var("NEXUS_ACP_TEST_CANCEL").as_deref() == Ok("1");
        let chat_mode = std::env::var("NEXUS_ACP_TEST_CHAT").as_deref() == Ok("1");
        assert!(!(chat_mode && cancel_mode));
        let idle_cancel = request_cancel();
        let (prompt, progress_seen, cancel_reply, duplicate_cancel) = if chat_mode {
            let turn = thread::spawn(|| command(Action::Chat("Xin chào Nexus".into())));
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let seen = loop {
                let snapshot = prompt_progress();
                if snapshot["running"] == true && snapshot["text"] == "Xin chào " { break true; }
                if std::time::Instant::now() >= deadline || turn.is_finished() { break false; }
                thread::sleep(Duration::from_millis(20));
            };
            (turn.join().expect("chat worker"), seen, Value::Null, Value::Null)
        } else if cancel_mode {
            let turn = thread::spawn(|| command(Action::Prompt));
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let progress_seen = loop {
                let snapshot = prompt_progress();
                if snapshot["running"] == true && snapshot["chunkCount"] == 1 { break true; }
                if std::time::Instant::now() >= deadline || turn.is_finished() { break false; }
                thread::sleep(Duration::from_millis(20));
            };
            let accepted = request_cancel();
            let duplicate = request_cancel();
            (turn.join().expect("prompt worker"), progress_seen, accepted, duplicate)
        } else {
            (command(Action::Prompt), false, Value::Null, Value::Null)
        };
        let finished = prompt_progress();
        let duplicate_prompt = command(Action::Prompt);
        let (followup, followup_progress, third, third_progress, extra_turns, over_limit) = if chat_mode {
            let second = command(Action::Chat("Bạn nhớ lời chào không?".into()));
            let second_progress = prompt_progress();
            let third = command(Action::Chat("Cảm ơn\n\nSelected file snapshot (untrusted text):\nlet value = 42\n".into()));
            let third_progress = prompt_progress();
            let mut extra=0;
            for turn in 4..=MAX_CHAT_TURNS {
                let r=command(Action::Chat(format!("Fixture turn {turn}")));
                let progress=prompt_progress();
                if r["ok"]!=true || r["responseId"]!=turn+2 || progress["responseId"]!=turn+2 ||
                    progress["text"].as_str().map(str::len)!=Some(4096) {break;}
                extra+=1;
            }
            let over_limit = command(Action::Chat("Twenty-first turn must be rejected".into()));
            (second, second_progress, third, third_progress, extra, over_limit)
        } else {
            (Value::Null, Value::Null, Value::Null, Value::Null, 0, Value::Null)
        };
        let close = command(Action::Close);
        let after_close = command(Action::New(NEW_REQUEST.to_vec()));
        let ok = first["ok"] == true && first["active"] == true && first["credentialMode"] == "gemini-api-key" &&
            second["ok"] == true && second["sessionCreated"] == true &&
            duplicate["ok"] == false && duplicate["active"] == true &&
            before_session["ok"] == false && before_session["active"] == true &&
            idle_cancel["ok"] == false && finished["running"] == false &&
            prompt["ok"] == true && (if chat_mode {
                progress_seen && prompt["chatCompleted"] == true && prompt["chunkCount"] == 2 &&
                finished["text"] == "Xin chào Nexus!" && finished["responseId"] == 3 &&
                followup["ok"] == true && followup["chatCompleted"] == true && followup["responseId"] == 4 &&
                followup_progress["responseId"] == 4 && followup_progress["text"] == "Nhớ lời chào Nexus." &&
                followup_progress["permissionDenials"] == 1 && followup_progress["toolUpdates"] == 2 &&
                third["ok"] == true && third["chatCompleted"] == true && third["responseId"] == 5 &&
                third_progress["responseId"] == 5 && third_progress["text"] == "Không có gì." &&
                third_progress["permissionDenials"] == 0 && third_progress["toolUpdates"] == 0 &&
                extra_turns==MAX_CHAT_TURNS-3 && over_limit["ok"] == false && over_limit["active"] == true
            } else if cancel_mode {
                progress_seen && cancel_reply["ok"] == true && duplicate_cancel["ok"] == false &&
                prompt["cancelled"] == true && prompt["stopReason"] == "cancelled" && prompt["chunkCount"] == 1
            } else { prompt["marker"] == SMOKE_REPLY && prompt["chunkCount"] == 2 }) &&
            duplicate_prompt["ok"] == false && duplicate_prompt["active"] == true &&
            close["ok"] == true && close["transportClosed"] == true && close["shutdownClean"] == true &&
            after_close["ok"] == false && after_close["active"] == false;
        let report = json!({"ok":ok,"fixtureOnly":true,"initialize":first,"newSession":second,
            "duplicate":duplicate,"beforeSession":before_session,"prompt":prompt,"duplicatePrompt":duplicate_prompt,"close":close,"afterClose":after_close,
            "cancelMode":cancel_mode,"chatMode":chat_mode,"progressSeen":progress_seen,"cancelRequest":cancel_reply,
            "duplicateCancel":duplicate_cancel,"idleCancel":idle_cancel,"finishedProgress":finished,
            "followup":followup,"followupProgress":followup_progress,"third":third,"thirdProgress":third_progress,"extraTurns":extra_turns,"overLimit":over_limit});
        std::fs::write(evidence, serde_json::to_vec_pretty(&report).unwrap()).expect("completed evidence");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert!(ok, "{report}");
    }
}
