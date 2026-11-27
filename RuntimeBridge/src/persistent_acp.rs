//! Local initialize/session-new lab. A dedicated worker owns the guest and
//! Tokio runtime; no raw pointer handles or concurrent requests cross FFI.
use crate::{process_probe::{self, ACPExchange}, ATTEMPTED};
use ish_embed_host::{IshInstance, SpawnOpts};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    ffi::{c_char, CStr, CString},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, mpsc::{self, Receiver, SyncSender}, Mutex},
    thread,
    time::Duration,
};

const IDLE: Duration = Duration::from_secs(60);
const REPLY: Duration = Duration::from_secs(300);
const NEW_REQUEST: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{\"cwd\":\"/tmp\",\"mcpServers\":[]}}\n";
type Reply = SyncSender<Value>;
enum Command { New(Vec<u8>, Reply), Close(Reply) }
static ACTIVE: Mutex<Option<SyncSender<Command>>> = Mutex::new(None);

enum Mode { Agy(String), #[cfg(test)] Fixture }

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

fn worker(path: PathBuf, mode: Mode, request: Vec<u8>, commands: Receiver<Command>, ready: Reply) {
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
    opts.envp = Some(env.into_iter().collect());
    opts.pipe_stdin = true;
    let session = match guest.spawn(opts) {
        Ok(s) => s, Err(_) => {
            let _ = rt.block_on(guest.shutdown());
            let _ = ready.send(failure("Agent spawn failed")); return;
        }
    };
    let mut exchange = ACPExchange::new(session);
    let init = rt.block_on(exchange.request(&request, 1));
    let response = match init {
        Ok(frame) => match &mode {
            Mode::Agy(_) => process_probe::initialize_projection(&frame),
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
    if ready.send(json!({"ok":true,"active":true,"response":response})).is_err() {
        let _ = rt.block_on(exchange.close());
        let _ = rt.block_on(guest.shutdown());
        return;
    }
    let mut created = false;
    let final_reply = loop {
        match commands.recv_timeout(IDLE) {
            Ok(Command::New(bytes, reply)) => {
                if created || !valid_new(&bytes) {
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
                        created = true;
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

fn start(path: PathBuf, mode: Mode, request: Vec<u8>) -> Value {
    if !process_probe::valid_initialize_request(&request) {
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
        .spawn(move || worker(path, mode, request, rx, reply)).is_err() {
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

fn command(bytes: Option<Vec<u8>>) -> Value {
    let Ok(mut slot) = ACTIVE.lock() else { return failure("Connection lock failed"); };
    let Some(tx) = slot.as_ref() else { return failure("No active connection"); };
    let (reply, wait) = mpsc::sync_channel(1);
    let message = match bytes { Some(b) => Command::New(b, reply), None => Command::Close(reply) };
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
            (Some(path), Some(sha), Some(req)) => start(PathBuf::from(path), Mode::Agy(sha), req.into_bytes()),
            _ => failure("Missing connection arguments"),
        }
    })
}

/// # Safety
/// request must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn nexus_acp_new_session(request: *const c_char) -> *mut c_char {
    ffi_result(|| match read_arg(request) {
        Some(req) if valid_new(req.as_bytes()) => command(Some(req.into_bytes())),
        _ => failure("Invalid session/new request"),
    })
}

#[no_mangle]
pub extern "C" fn nexus_acp_close() -> *mut c_char {
    ffi_result(|| command(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricted_request_and_session_shape() {
        assert!(valid_new(NEW_REQUEST));
        assert!(!valid_new(b""));
        assert!(!valid_new(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/prompt\"}\n"));
        assert!(!session_created(&json!({"result":{"sessionId":""}})));
        assert!(session_created(&json!({"result":{"sessionId":"fixture-1"}})));
    }

    #[test]
    #[ignore = "requires a separate fresh Ubuntu fakefs"]
    fn real_persistent_acp_transport() {
        let path = std::env::var_os("NEXUS_ACP_ROOTFS").expect("set NEXUS_ACP_ROOTFS");
        let evidence = std::env::var_os("NEXUS_ACP_EVIDENCE").expect("set NEXUS_ACP_EVIDENCE");
        let first = start(PathBuf::from(path), Mode::Fixture, process_probe::AGY_INITIALIZE_REQUEST.to_vec());
        let second = command(Some(NEW_REQUEST.to_vec()));
        let duplicate = command(Some(NEW_REQUEST.to_vec()));
        let close = command(None);
        let after_close = command(Some(NEW_REQUEST.to_vec()));
        let ok = first["ok"] == true && first["active"] == true &&
            second["ok"] == true && second["sessionCreated"] == true &&
            duplicate["ok"] == false && duplicate["active"] == true &&
            close["ok"] == true && close["transportClosed"] == true && close["shutdownClean"] == true &&
            after_close["ok"] == false && after_close["active"] == false;
        let report = json!({"ok":ok,"fixtureOnly":true,"initialize":first,"newSession":second,
            "duplicate":duplicate,"close":close,"afterClose":after_close});
        std::fs::write(evidence, serde_json::to_vec_pretty(&report).unwrap()).expect("completed evidence");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert!(ok, "{report}");
    }
}
