//! Separate local jobs, never run concurrently with ACP or another boot.
use crate::{process_probe, ATTEMPTED};
use ish_embed_host::IshInstance;
use serde_json::{json, Value};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{collections::{HashMap,VecDeque}, ffi::{c_char,CStr,CString}, path::Path,
    sync::{Mutex,atomic::Ordering}};

pub(crate) enum Control { Input(Vec<u8>), Resize(u16,u16), Stop }
struct State {
    running: bool, ready: bool, mode: u8, output: String, raw: Vec<u8>, total: u64,
    controls: VecDeque<Control>, stop: bool,
}
static STATE: Mutex<State> = Mutex::new(State {
    running:false, ready:false, mode:0, output:String::new(), raw:Vec::new(), total:0, controls:VecDeque::new(), stop:false,
});
const WORKBENCH: &str = "umask 077; mkdir -p /tmp/nexus-workbench /tmp/nexus-vscode-data /tmp/nexus-vscode-extensions && exec /opt/nexus/workbench/bin/openvscode-server --host 127.0.0.1 --port 18765 --connection-token \"$NEXUS_WORKBENCH_TOKEN\" --server-data-dir /tmp/nexus-vscode-data --extensions-dir /tmp/nexus-vscode-extensions /tmp/nexus-workbench";
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
fn valid_token(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) }
fn progress() -> Value {
    STATE.lock().map(|s| json!({"ok":true,"running":s.running,"ready":s.ready,"output":s.output,"stopRequested":s.stop}))
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
        if let Ok(mut s) = STATE.lock() { s.running = false; s.ready = false; s.controls.clear(); }
    }
}
fn run(path: &Path, mode: u8, input: &str) -> Value {
    let call=if mode==4 {match parse_call(input) {Some(v)=>Some(v),None=>return fail("Invalid MCP call")}} else {None};
    if !matches!(mode,1..=4) || (mode == 3 && !valid_token(input)) ||
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
    *s = State { running:true,ready:false,mode,output:String::new(),raw:Vec::new(),total:0,controls:VecDeque::new(),stop:false };
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
        ("TERM".into(),if mode==1 {"xterm-256color".into()} else {"dumb".into()}),("NODE_OPTIONS".into(),"--jitless".into())]);
    let libc = rt.block_on(process_probe::glibc_compat(&guest,&env));
    let mut result = if libc["passed"] != true { fail("Ubuntu compatibility failed") }
        else if let Some(call)=call.as_ref() {rt.block_on(process_probe::mcp_job(&guest,&env,call["command"].as_str().unwrap(),Some((call["name"].as_str().unwrap(),&call["arguments"]))))}
        else if mode == 2 { rt.block_on(process_probe::mcp_discover(&guest,&env,input)) }
        else {
            if mode == 3 { env.insert("NEXUS_WORKBENCH_TOKEN".into(),input.into()); }
            rt.block_on(process_probe::local_process(&guest,&env,if mode == 3 { WORKBENCH } else { input },mode == 3))
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
        assert!(!valid_token("token"));
        assert!(parse_call(r#"{"command":"exec server","name":"echo","arguments":{}}"#).is_some());
        assert!(parse_call(r#"{"command":"exec server","name":"echo","arguments":[]}"#).is_none());
        assert!(parse_call(r#"{"command":"exec server","name":"echo","arguments":{},"extra":true}"#).is_none());
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
