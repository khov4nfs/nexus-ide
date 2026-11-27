//! Bounded process lifecycle probes against the pinned real iSH host API.
//! No caller-supplied commands or untrusted files are exposed.
use ish_embed_host::{IshInstance, IshSession, PtySize, SpawnOpts, Stream, WriteStatus};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

const LIMIT: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Capture {
    cursor: u64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pty: Vec<u8>,
    closed: bool,
    exited: bool,
    code: Option<i32>,
    signal: Option<i32>,
}
impl Capture {
    fn append(&mut self, seq: u64, stream: Stream, data: &[u8]) -> Result<(), String> {
        if seq <= self.cursor {
            return Err("nonmonotonic output sequence".into());
        }
        if self.stdout.len() + self.stderr.len() + self.pty.len() + data.len() > LIMIT {
            return Err("probe output exceeded 64 KiB".into());
        }
        match stream {
            Stream::Stdout => self.stdout.extend_from_slice(data),
            Stream::Stderr => self.stderr.extend_from_slice(data),
            Stream::Pty => self.pty.extend_from_slice(data),
        }
        self.cursor = seq;
        Ok(())
    }
    async fn poll(&mut self, session: &IshSession, deadline: Instant) -> Result<(), String> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("process deadline exceeded".into());
        }
        let read = session.read(Some(self.cursor), Some(LIMIT), Some(remaining.as_millis().min(100).max(1) as u64))
            .await.map_err(|e| e.to_string())?;
        if read.missed_older_bytes {
            return Err("guest output retention gap".into());
        }
        let empty = read.chunks.is_empty();
        for chunk in read.chunks {
            self.append(chunk.seq, chunk.stream, &chunk.bytes)?;
        }
        // Exited is not final: upstream can emit additional output before Closed.
        // Drain until Closed AND no more chunks; don't advance via next_seq.
        self.closed = read.closed && empty;
        self.exited = read.exited;
        self.code = read.exit_code;
        self.signal = read.term_signal;
        Ok(())
    }
    fn evidence(&self) -> Value {
        json!({"stdout":String::from_utf8_lossy(&self.stdout),
               "stderr":String::from_utf8_lossy(&self.stderr),
               "pty":String::from_utf8_lossy(&self.pty),
               "exitCode":self.code,"termSignal":self.signal,"closed":self.closed})
    }
}
fn contains(data: &[u8], marker: &[u8]) -> bool {
    !marker.is_empty() && data.windows(marker.len()).any(|w| w == marker)
}
async fn wait_ready<F>(session: &IshSession, capture: &mut Capture, deadline: Instant, ready: F) -> Result<(), String>
where F: Fn(&Capture) -> bool {
    loop {
        capture.poll(session, deadline).await?;
        if capture.exited || capture.closed {
            return Err("guest exited before input/termination".into());
        }
        if ready(capture) { return Ok(()); }
    }
}
async fn write_ready(session: &IshSession, bytes: &[u8], deadline: Instant) -> Result<(), String> {
    loop {
        if Instant::now() >= deadline { return Err("stdin readiness deadline exceeded".into()); }
        match session.write(bytes).await.map_err(|e| e.to_string())? {
            WriteStatus::Accepted => return Ok(()),
            WriteStatus::Starting => tokio::time::sleep(Duration::from_millis(20)).await,
            other => return Err(format!("stdin rejected: {other:?}")),
        }
    }
}
async fn drain(session: &IshSession, capture: &mut Capture, deadline: Instant) -> Result<(), String> {
    while !capture.closed { capture.poll(session, deadline).await?; }
    Ok(())
}
fn shell(script: &str, env: &HashMap<String,String>) -> SpawnOpts {
    let mut opts = SpawnOpts::cmd(["/bin/sh","-c",script]);
    opts.cwd = Some(Path::new("/").to_path_buf());
    opts.envp = Some(env.iter().map(|(k,v)| (k.clone(),v.clone())).collect());
    opts.pipe_stdin = true;
    opts
}
async fn streams(guest: &IshInstance, env: &HashMap<String,String>) -> Result<Value,String> {
    let session = guest.spawn(shell("printf 'NEXUS_OUT_READY\\n'; printf 'NEXUS_ERR_READY\\n' >&2; IFS= read -r line || exit 8; printf 'reply:%s\\n' \"$line\"",env))
        .map_err(|e| e.to_string())?;
    let result = async {
        let deadline = Instant::now() + TIMEOUT;
        let mut capture = Capture::default();
        wait_ready(&session,&mut capture,deadline,|c|
            contains(&c.stdout,b"NEXUS_OUT_READY") && contains(&c.stderr,b"NEXUS_ERR_READY")).await?;
        write_ready(&session,b"nexus-ping\n",deadline).await?;
        drain(&session,&mut capture,deadline).await?;
        if capture.code != Some(0) || capture.signal.unwrap_or(0) != 0 ||
            !contains(&capture.stdout,b"reply:nexus-ping\n") ||
            contains(&capture.stdout,b"NEXUS_ERR_READY") ||
            contains(&capture.stderr,b"NEXUS_OUT_READY") || !capture.pty.is_empty() {
            return Err(format!("stdio mismatch: {}",capture.evidence()));
        }
        Ok(capture.evidence())
    }.await;
    if result.is_err() { let _ = session.terminate().await; }
    result
}
async fn terminal(guest: &IshInstance, env: &HashMap<String,String>) -> Result<Value,String> {
    let mut opts = shell("printf 'NEXUS_PTY_READY\\n'; IFS= read -r line || exit 8; stty size && printf 'NEXUS_PTY_TAIL\\n'",env);
    opts.tty = true;
    opts.pipe_stdin = false;
    opts.size = Some(PtySize{cols:80,rows:24});
    let session = guest.spawn(opts).map_err(|e| e.to_string())?;
    let result = async {
        let deadline = Instant::now() + TIMEOUT;
        let mut capture = Capture::default();
        wait_ready(&session,&mut capture,deadline,|c| contains(&c.pty,b"NEXUS_PTY_READY")).await?;
        session.resize(111,33).await.map_err(|e| e.to_string())?;
        write_ready(&session,b"go\n",deadline).await?;
        drain(&session,&mut capture,deadline).await?;
        if capture.code != Some(0) || capture.signal.unwrap_or(0) != 0 ||
            !String::from_utf8_lossy(&capture.pty).lines().any(|line| line.trim()=="33 111") ||
            !contains(&capture.pty,b"NEXUS_PTY_TAIL") {
            return Err(format!("PTY resize mismatch: {}",capture.evidence()));
        }
        Ok(capture.evidence())
    }.await;
    if result.is_err() { let _ = session.terminate().await; }
    result
}
async fn cancellation(guest: &IshInstance, env: &HashMap<String,String>) -> Result<Value,String> {
    let session = guest.spawn(shell("printf 'NEXUS_CANCEL_READY\\n'; exec /bin/sleep 600",env))
        .map_err(|e| e.to_string())?;
    let result = async {
        let deadline = Instant::now() + TIMEOUT;
        let mut capture = Capture::default();
        wait_ready(&session,&mut capture,deadline,|c| contains(&c.stdout,b"NEXUS_CANCEL_READY")).await?;
        session.terminate().await.map_err(|e| e.to_string())?;
        drain(&session,&mut capture,deadline).await?;
        if capture.signal != Some(9) || !capture.exited {
            return Err(format!("cancellation mismatch: {}",capture.evidence()));
        }
        Ok(capture.evidence())
    }.await;
    if result.is_err() { let _ = session.terminate().await; }
    result
}

pub async fn run(guest: &IshInstance, env: &HashMap<String,String>) -> Vec<Value> {
    let mut rows = Vec::new();
    for name in ["stdio_roundtrip","pty_resize","process_cancel"] {
        let result = match name {
            "stdio_roundtrip" => streams(guest,env).await,
            "pty_resize" => terminal(guest,env).await,
            _ => cancellation(guest,env).await,
        };
        match result {
            Ok(evidence) => rows.push(json!({"name":name,"passed":true,"evidence":evidence})),
            Err(error) => {
                rows.push(json!({"name":name,"passed":false,"error":error}));
                break;
            }
        }
    }
    rows
}


// Fixed official Alpine package gate. No caller-supplied names, URLs,
// shell fragments, credentials or user workspace. TLS and APK signature
// verification stay enabled. No package install scripts/triggers.
const PACKAGE_SCRIPT: &str = r#"
set -eu
if /sbin/apk info -e jq >/dev/null 2>&1; then
    echo 'Refusing preinstalled jq: need a fresh rootfs' >&2
    exit 31
fi
printf 'NEXUS_PACKAGE_ABSENT\n'
# Upstream minirootfs may not provide DNS. Only in the explicit network probe.
if [ ! -s /etc/resolv.conf ]; then
    printf 'nameserver 1.1.1.1\n' > /etc/resolv.conf
    printf 'NEXUS_DNS_FALLBACK\n'
fi
/sbin/apk --repositories-file /dev/null \
    --repository https://dl-cdn.alpinelinux.org/alpine/v3.21/main \
    --no-cache --no-scripts add jq >&2
/sbin/apk info -e jq >/dev/null
printf 'NEXUS_PACKAGE_INSTALLED\n'
# Query installed package versions without a package argument.
installed=$(/sbin/apk --no-network info -v)
pkg=$(printf '%s\n' "$installed" | grep -E '^jq-[0-9][^[:space:]]*-r[0-9]+$')
bin=$(/usr/bin/jq --version)
printf 'NEXUS_APK_VERSION=%s\nNEXUS_BINARY_VERSION=%s\n' "$pkg" "$bin"
printf '%s\n' '{"value":"NEXUS_JQ_OK"}' | /usr/bin/jq -er '.value'
"#;

pub async fn package_install(guest: &IshInstance, env: &HashMap<String,String>) -> Value {
    let name = "apk_install_jq";
    let mut opts = shell(PACKAGE_SCRIPT, env);
    opts.pipe_stdin = false;
    let session = match guest.spawn(opts) {
        Ok(s) => s,
        Err(e) => return json!({"name":name,"passed":false,"error":e.to_string()}),
    };
    let mut capture = Capture::default();
    let deadline = Instant::now() + Duration::from_secs(180);
    let result = drain(&session, &mut capture, deadline).await;
    if result.is_err() { let _ = session.terminate().await; }
    let passed = result.is_ok() && capture.code == Some(0) &&
        capture.signal.unwrap_or(0) == 0 && package_output_ok(&capture.stdout) &&
        capture.pty.is_empty();
    json!({"name":name,"passed":passed,"evidence":capture.evidence(),
           "repository":"https://dl-cdn.alpinelinux.org/alpine/v3.21/main",
           "error":result.err()})
}

fn package_output_ok(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let absent = lines.iter().position(|line| *line == "NEXUS_PACKAGE_ABSENT");
    let installed = lines.iter().position(|line| *line == "NEXUS_PACKAGE_INSTALLED");
    let (Some(a), Some(b)) = (absent, installed) else { return false; };
    if a >= b || lines.len() != b + 4 || lines.last() != Some(&"NEXUS_JQ_OK") {
        return false;
    }
    let Some(pkg) = lines[b+1].strip_prefix("NEXUS_APK_VERSION=") else { return false; };
    let Some(bin) = lines[b+2].strip_prefix("NEXUS_BINARY_VERSION=") else { return false; };
    let Some((version, revision)) = pkg.rsplit_once("-r") else { return false; };
    version == bin && version.strip_prefix("jq-")
        .map(|v| v.as_bytes().first().map_or(false, |c| c.is_ascii_digit()) &&
            v.bytes().all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c)))
        .unwrap_or(false) &&
        !revision.is_empty() && revision.bytes().all(|c| c.is_ascii_digit())
}

const TOOLCHAIN_SCRIPT: &str = include_str!("../fixtures/toolchains.sh");

pub async fn toolchain_install(guest: &IshInstance, env: &HashMap<String,String>) -> Value {
    let name = "development_toolchains";
    let mut opts = shell(TOOLCHAIN_SCRIPT, env);
    opts.pipe_stdin = false;
    let session = match guest.spawn(opts) {
        Ok(s) => s,
        Err(e) => return json!({"name":name,"passed":false,"error":e.to_string()}),
    };
    let mut capture = Capture::default();
    let result = drain(&session, &mut capture, Instant::now() + Duration::from_secs(600)).await;
    if result.is_err() { let _ = session.terminate().await; }
    let passed = result.is_ok() && capture.code == Some(0) &&
        capture.signal == Some(0) && capture.closed && capture.pty.is_empty() &&
        toolchain_output_ok(&capture.stdout) &&
        contains(&capture.stderr, b"NEXUS_GUEST_ENTROPY_OK\n");
    json!({"name":name,"passed":passed,"evidence":capture.evidence(),"error":result.err()})
}

fn toolchain_output_ok(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != 13 || lines[0] != "NEXUS_TOOLCHAINS_ABSENT" ||
        lines[9] != "NEXUS_NODE_DEPENDENCY_OK" ||
        lines[10] != "NEXUS_PYTHON_TEST_OK" ||
        lines[12] != "NEXUS_TOOLCHAINS_OK" { return false; }
    for (line, name) in lines[1..5].iter().zip(["nodejs", "npm", "python3", "git"]) {
        let prefix = format!("NEXUS_TOOLCHAIN_APK={name}-");
        let Some(rest) = line.strip_prefix(prefix.as_str()) else { return false; };
        let Some((version, revision)) = rest.rsplit_once("-r") else { return false; };
        if !version.as_bytes().first().map_or(false, |c| c.is_ascii_digit()) ||
           !version.bytes().all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c)) ||
           revision.is_empty() || !revision.bytes().all(|c| c.is_ascii_digit()) { return false; }
    }
    for (line, prefix) in lines[5..9].iter().zip(["NEXUS_NODE_VERSION=v", "NEXUS_NPM_VERSION=", "NEXUS_PYTHON_VERSION=Python ", "NEXUS_GIT_VERSION=git version "]) {
        let Some(version) = line.strip_prefix(prefix) else { return false; };
        if !version.as_bytes().first().map_or(false, |c| c.is_ascii_digit()) { return false; }
    }
    let Some(hash) = lines[11].strip_prefix("NEXUS_GIT_COMMIT=") else { return false; };
    hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}


// A bounded text transfer over the existing guest stdin/stdout transport.
// Only a disposable guest file is touched; no host filesystem mount.
pub const WORKSPACE_LIMIT: usize = 2048;
pub const WORKSPACE_FIXTURE: &str = "Nexus workspace\nTiếng Việt — UTF-8\nliteral: $(echo never-execute) `quote`\\\n";
const WORKSPACE_SUFFIX: &str = "\n[Nexus guest workspace probe]\n";
const WORKSPACE_SCRIPT: &str = include_str!("../fixtures/workspace.sh");

pub fn workspace_input_ok(text: &str) -> bool {
    text.len() <= WORKSPACE_LIMIT && !text.contains('\0')
}

pub async fn workspace_roundtrip(guest: &IshInstance, env: &HashMap<String,String>, text: &str) -> Value {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let name = "workspace_roundtrip";
    if !workspace_input_ok(text) {
        return json!({"name":name,"passed":false,"error":"Invalid workspace input"});
    }
    let session = match guest.spawn(shell(WORKSPACE_SCRIPT, env)) {
        Ok(s) => s,
        Err(e) => return json!({"name":name,"passed":false,"error":e.to_string()}),
    };
    let deadline = Instant::now() + TIMEOUT;
    let mut capture = Capture::default();
    let result = async {
        wait_ready(&session, &mut capture, deadline, |c| contains(&c.stderr,b"NEXUS_WORKSPACE_READY\n")).await?;
        let wire = format!("{}\n", STANDARD.encode(text.as_bytes()));
        write_ready(&session, wire.as_bytes(), deadline).await?;
        drain(&session, &mut capture, deadline).await?;
        if capture.code != Some(0) || capture.signal != Some(0) || !capture.closed ||
           !capture.pty.is_empty() || capture.stdout != format!("{text}{WORKSPACE_SUFFIX}").as_bytes() {
            return Err("workspace byte roundtrip mismatch".to_owned());
        }
        Ok(())
    }.await;
    if result.is_err() { let _ = session.terminate().await; }
    // Do not return raw text from a failed transfer.
    match result {
        Ok(()) => json!({"name":name,"passed":true,"evidence":capture.evidence(),"error":null}),
        Err(error) => json!({"name":name,"passed":false,"error":error}),
    }
}


// Separate compatibility lane for glibc and the official, user-provisioned
// AGY binary. No model requests, project mounts or authentication.
const GLIBC_SCRIPT: &str = r#"
set -eu
/usr/bin/uname -m
/usr/bin/getconf GNU_LIBC_VERSION
# apt-get dynamically links libstdc++; require a real successful loader run.
/usr/bin/apt-get --version >/dev/null
test -r /usr/lib/aarch64-linux-gnu/libstdc++.so.6
/bin/grep -a -q 'GLIBCXX_3\.4\.25' /usr/lib/aarch64-linux-gnu/libstdc++.so.6
printf 'NEXUS_GLIBCXX_OK\n'
"#;

fn glibc_output_ok(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else { return false; };
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 3 || lines[0] != "aarch64" || lines[2] != "NEXUS_GLIBCXX_OK" { return false; }
    let Some(version) = lines[1].strip_prefix("glibc ") else { return false; };
    let Some((major, minor)) = version.split_once('.') else { return false; };
    matches!((major.parse::<u32>(), minor.parse::<u32>()), (Ok(a), Ok(b)) if (a,b) >= (2,28))
}

pub async fn glibc_compat(guest: &IshInstance, env: &HashMap<String,String>) -> Value {
    let name = "glibc_compat";
    let mut opts = shell(GLIBC_SCRIPT, env);
    opts.pipe_stdin = false;
    let session = match guest.spawn(opts) {
        Ok(s) => s,
        Err(e) => return json!({"name":name,"passed":false,"error":e.to_string()}),
    };
    let mut capture = Capture::default();
    let result = drain(&session, &mut capture, Instant::now() + Duration::from_secs(60)).await;
    if result.is_err() { let _ = session.terminate().await; }
    let passed = result.is_ok() && capture.code == Some(0) && capture.signal == Some(0)
        && capture.closed && capture.pty.is_empty() && glibc_output_ok(&capture.stdout);
    json!({"name":name,"passed":passed,"evidence":capture.evidence(),"error":result.err()})
}

// Configure the documented provider without writing any credential.
pub(crate) async fn prepare_gemini_provider(guest: &IshInstance, env: &HashMap<String,String>) -> bool {
    let script = r#"
set -eu
umask 077
test ! -e /root/.gemini
mkdir -p /root/.gemini/antigravity-cli
printf '%s\n' '{"modelProvider":"gemini"}' > /root/.gemini/antigravity-cli/settings.json
"#;
    let mut opts = shell(script, env);
    opts.pipe_stdin = false;
    let session = match guest.spawn(opts) { Ok(s) => s, Err(_) => return false };
    let mut capture = Capture::default();
    let result = drain(&session, &mut capture, Instant::now() + TIMEOUT).await;
    if result.is_err() { let _ = session.terminate().await; }
    result.is_ok() && capture.closed && capture.code == Some(0) && capture.signal == Some(0)
        && capture.stdout.is_empty() && capture.stderr.is_empty() && capture.pty.is_empty()
}

pub fn agy_digest_ok(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub async fn agy_binary_digest(guest: &IshInstance, env: &HashMap<String,String>, expected: &str) -> Value {
    let name = "agy_binary_digest";
    let mut opts = SpawnOpts::cmd(["/usr/bin/sha256sum", "/opt/nexus/agy"]);
    opts.cwd = Some(Path::new("/").to_path_buf());
    opts.envp = Some(env.iter().map(|(k,v)| (k.clone(),v.clone())).collect());
    let session = match guest.spawn(opts) {
        Ok(s) => s,
        Err(e) => return json!({"name":name,"passed":false,"error":e.to_string()}),
    };
    let mut capture = Capture::default();
    let result = drain(&session, &mut capture, Instant::now() + Duration::from_secs(120)).await;
    if result.is_err() { let _ = session.terminate().await; }
    let passed = result.is_ok() && capture.code == Some(0) && capture.signal == Some(0)
        && capture.closed && capture.pty.is_empty()
        && capture.stdout == format!("{expected}  /opt/nexus/agy\n").as_bytes();
    json!({"name":name,"passed":passed,"sha256":expected,"closed":capture.closed,
           "error":if passed { None } else { Some("Guest executable digest check failed") }})
}

pub const AGY_INITIALIZE_REQUEST: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{},\"clientInfo\":{\"name\":\"nexus-ios\",\"version\":\"0.3.2\"}}}\n";

pub fn valid_initialize_request(bytes: &[u8]) -> bool {
    if bytes.len() > 4096 || bytes.last() != Some(&b'\n') ||
        bytes[..bytes.len()-1].contains(&b'\n') { return false; }
    let Ok(frame) = serde_json::from_slice::<Value>(bytes) else { return false; };
    let expected: Value = serde_json::from_slice(AGY_INITIALIZE_REQUEST).expect("fixed request");
    frame == expected
}

// Project only the bounded protocol fields Swift needs. Never expose raw agent
// notifications, authentication data, stderr or arbitrary error messages.
pub(crate) fn initialize_projection(frame: &Value) -> Option<Value> {
    if frame["jsonrpc"] != "2.0" || frame["id"] != 1 || frame.get("method").is_some() {
        return None;
    }
    match (frame.get("result"), frame.get("error")) {
        (Some(_), None) if agy_initialize_ok(frame) => {
            let info = &frame["result"]["agentInfo"];
            let name = info["name"].as_str()?;
            let version = info["version"].as_str().unwrap_or("unknown");
            if name.len() > 256 || version.len() > 256 { return None; }
            Some(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,
                "agentInfo":{"name":name,"version":version}}}))
        }
        (None, Some(error)) => {
            let code = error["code"].as_i64()?;
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&code) { return None; }
            let auth = code == -32000 && error["message"].as_str()
                .map(|s| s.eq_ignore_ascii_case("Authentication required")).unwrap_or(false);
            Some(json!({"jsonrpc":"2.0","id":1,"error":{"code":code,
                "message":if auth {"Authentication required"} else {"Agent rejected initialize"}}}))
        }
        _ => None,
    }
}

// Do not treat arbitrary JSON or a JSON-RPC error as a successful initialize.
fn agy_initialize_ok(frame: &Value) -> bool {
    frame["jsonrpc"] == "2.0" && frame["id"] == 1 &&
        frame.get("error").is_none() && frame["result"]["protocolVersion"] == 1 &&
        frame["result"]["agentInfo"]["name"].as_str()
            .map(|s| s.to_ascii_lowercase().contains("antigravity")).unwrap_or(false)
}

pub async fn agy_initialize(guest: &IshInstance, env: &HashMap<String,String>, request: &[u8]) -> Value {
    let name = "agy_acp_initialize";
    // Fixed guest-only path; prepared by the developer from the official
    // artifact. Never substitute a fixture or run a host binary.
    let mut opts = SpawnOpts::cmd(["/opt/nexus/agy", "acp", "--disable-telemetry"]);
    opts.cwd = Some(Path::new("/tmp").to_path_buf());
    let mut agent_env = env.clone();
    agent_env.insert("AGY_CLI_DISABLE_AUTO_UPDATE".into(), "true".into());
    opts.envp = Some(agent_env.into_iter().collect());
    opts.pipe_stdin = true;
    let session = match guest.spawn(opts) {
        Ok(s) => s,
        Err(e) => return json!({"name":name,"passed":false,"error":e.to_string()}),
    };
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut capture = Capture::default();
    let result: Result<Value,String> = async {
        write_ready(&session, request, deadline).await?;
        let mut cursor = 0;
        loop {
            capture.poll(&session, deadline).await?;
            while let Some(end) = capture.stdout[cursor..].iter().position(|b| *b == b'\n') {
                let end = cursor + end;
                let frame: Value = serde_json::from_slice(&capture.stdout[cursor..end])
                    .map_err(|_| "non-JSON ACP stdout".to_owned())?;
                cursor = end + 1;
                if frame.get("id").is_some() {
                    return initialize_projection(&frame)
                        .ok_or_else(|| "Unexpected initialize response or server request".to_owned());
                }
                if frame["jsonrpc"] != "2.0" || !frame["method"].is_string() {
                    return Err("Unexpected ACP notification".into());
                }
            }
            if capture.exited || capture.closed { return Err("AGY exited before initialize".into()); }
        }
    }.await;
    // No session/new or prompt. Kill only this probe's process group.
    let terminated = session.terminate().await.is_ok();
    let drained = drain(&session, &mut capture, Instant::now() + TIMEOUT).await.is_ok();
    match result {
        Ok(response) => {
            let clean = terminated && drained && capture.closed && capture.pty.is_empty();
            let success = response.get("result").is_some();
            let evidence = if success { json!({"protocolVersion":response["result"]["protocolVersion"],
                "agentName":response["result"]["agentInfo"]["name"],
                "agentVersion":response["result"]["agentInfo"]["version"]}) } else { Value::Null };
            json!({"name":name,"passed":clean && success,"evidence":evidence,
                "response":response,"closed":capture.closed,"transportClean":clean})
        },
        Err(error) => json!({"name":name,"passed":false,"error":error,"closed":capture.closed}),
    }
}


// Bounded, serial request/response exchange over one long-lived guest process.
// A retention gap, unknown request, unexpected response, or EOF poisons the
// connection. Only the owner worker may use this object.
pub(crate) struct ACPExchange {
    session: std::sync::Arc<IshSession>,
    capture: Capture,
    offset: usize,
}
impl ACPExchange {
    pub(crate) fn new(session: std::sync::Arc<IshSession>) -> Self {
        Self { session, capture: Capture::default(), offset: 0 }
    }

    pub(crate) async fn request(&mut self, bytes: &[u8], id: i64) -> Result<Value,String> {
        self.request_updates(bytes, id, |_| Ok(())).await
    }

    pub(crate) async fn request_updates<F>(&mut self, bytes: &[u8], id: i64, update: F) -> Result<Value,String>
    where F: FnMut(&Value) -> Result<(),String> {
        self.request_controlled(bytes, id, update, || None).await
    }

    // Control is checked by the owner of the same stdin stream, never by
    // another writer. Cancellation is a notification, not a new request ID.
    pub(crate) async fn request_controlled<F, C>(&mut self, bytes: &[u8], id: i64, mut update: F, mut control: C) -> Result<Value,String>
    where F: FnMut(&Value) -> Result<(),String>, C: FnMut() -> Option<Vec<u8>> {
        let mut deadline = Instant::now() + Duration::from_secs(90);
        write_ready(&self.session, bytes, deadline).await?;
        loop {
            if let Some(cancel) = control() {
                deadline = deadline.min(Instant::now() + Duration::from_secs(10));
                write_ready(&self.session, &cancel, deadline).await?;
            }
            self.capture.poll(&self.session, deadline).await?;
            if self.capture.exited || self.capture.closed || !self.capture.pty.is_empty() {
                return Err("ACP process stopped or wrong stream".into());
            }
            let mut answer = None;
            while let Some(end) = self.capture.stdout[self.offset..].iter().position(|b| *b == b'\n') {
                let end = self.offset + end;
                let frame: Value = serde_json::from_slice(&self.capture.stdout[self.offset..end])
                    .map_err(|_| "Invalid ACP stdout".to_owned())?;
                self.offset = end + 1;
                if frame["jsonrpc"] != "2.0" { return Err("Invalid JSON-RPC version".into()); }
                if frame.get("id").is_none() && frame["method"].is_string() &&
                    frame.get("result").is_none() && frame.get("error").is_none() {
                    update(&frame)?;
                    continue;
                }
                if frame.get("method").is_some() || frame["id"].as_i64() != Some(id) ||
                    frame.get("result").is_some() == frame.get("error").is_some() ||
                    answer.is_some() {
                    return Err("Unexpected ACP request or response".into());
                }
                answer = Some(frame);
            }
            if let Some(frame) = answer {
                if self.offset != self.capture.stdout.len() {
                    return Err("Trailing incomplete ACP frame".into());
                }
                return Ok(frame);
            }
        }
    }

    pub(crate) async fn close(&mut self) -> bool {
        let sent = self.session.terminate().await.is_ok();
        let drained = drain(&self.session, &mut self.capture, Instant::now() + TIMEOUT).await.is_ok();
        sent && drained && self.capture.closed && self.capture.pty.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_initialize_request_and_projection_fail_closed() {
        assert!(valid_initialize_request(AGY_INITIALIZE_REQUEST));
        assert!(!valid_initialize_request(b""));
        assert!(!valid_initialize_request(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session/new\"}\n"));
        let mut two = AGY_INITIALIZE_REQUEST.to_vec();
        two.extend_from_slice(AGY_INITIALIZE_REQUEST);
        assert!(!valid_initialize_request(&two));
        let error = json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required","data":{"secret":"discard"}}});
        let projected = initialize_projection(&error).unwrap();
        assert_eq!(projected["error"]["message"], "Authentication required");
        assert!(projected["error"].get("data").is_none());
        assert!(initialize_projection(&json!({"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{}})).is_none());
        assert!(initialize_projection(&json!({"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}})).is_none());
    }
    #[test]
    fn compatibility_requires_glibc_and_real_agent_identity() {
        assert!(glibc_output_ok(b"aarch64\nglibc 2.39\nNEXUS_GLIBCXX_OK\n"));
        assert!(!glibc_output_ok(b"aarch64\nglibc 2.27\nNEXUS_GLIBCXX_OK\n"));
        assert!(!glibc_output_ok(b"aarch64\nmusl 1.2\nNEXUS_GLIBCXX_OK\n"));
        let good = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"Antigravity ACP Server","version":"test"}}});
        assert!(agy_initialize_ok(&good));
        assert!(!agy_initialize_ok(&json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000}})));
        assert!(!agy_initialize_ok(&json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"fixture"}}})));
    }
    #[test]
    fn workspace_input_boundaries() {
        assert!(workspace_input_ok(WORKSPACE_FIXTURE));
        assert!(workspace_input_ok(""));
        assert!(workspace_input_ok(&"a".repeat(WORKSPACE_LIMIT)));
        assert!(!workspace_input_ok(&"a".repeat(WORKSPACE_LIMIT+1)));
        assert!(!workspace_input_ok("a\0b"));
        assert!(!workspace_input_ok(&"é".repeat(WORKSPACE_LIMIT)));
    }
    #[test]
    fn toolchain_evidence_requires_all_workflow_steps() {
        let good = "NEXUS_TOOLCHAINS_ABSENT\nNEXUS_TOOLCHAIN_APK=nodejs-22.1.0-r0\nNEXUS_TOOLCHAIN_APK=npm-10.1.0-r0\nNEXUS_TOOLCHAIN_APK=python3-3.12.0-r0\nNEXUS_TOOLCHAIN_APK=git-2.47.0-r0\nNEXUS_NODE_VERSION=v22.1.0\nNEXUS_NPM_VERSION=10.1.0\nNEXUS_PYTHON_VERSION=Python 3.12.0\nNEXUS_GIT_VERSION=git version 2.47.0\nNEXUS_NODE_DEPENDENCY_OK\nNEXUS_PYTHON_TEST_OK\nNEXUS_GIT_COMMIT=0123456789abcdef0123456789abcdef01234567\nNEXUS_TOOLCHAINS_OK\n";
        assert!(toolchain_output_ok(good.as_bytes()));
        assert!(!toolchain_output_ok(good.replace("NEXUS_PYTHON_TEST_OK\n", "").as_bytes()));
        assert!(!toolchain_output_ok(good.replace("NEXUS_GIT_COMMIT=0", "NEXUS_GIT_COMMIT=z").as_bytes()));
        assert!(!toolchain_output_ok(good.replace("npm-10.1.0-r0", "npm-10.1.0").as_bytes()));
    }
    #[test]
    fn package_evidence_requires_install_and_execution() {
        let good = b"NEXUS_PACKAGE_ABSENT\nNEXUS_DNS_FALLBACK\nNEXUS_PACKAGE_INSTALLED\nNEXUS_APK_VERSION=jq-1.8.2-r0\nNEXUS_BINARY_VERSION=jq-1.8.2\nNEXUS_JQ_OK\n";
        assert!(package_output_ok(good));
        assert!(!package_output_ok(b"NEXUS_JQ_OK\n"));
        assert!(!package_output_ok(b"NEXUS_PACKAGE_INSTALLED\nNEXUS_PACKAGE_ABSENT\nNEXUS_APK_VERSION=jq-1.8.2-r0\nNEXUS_BINARY_VERSION=jq-1.8.2\nNEXUS_JQ_OK\n"));
        // Regression: actual stdout from failed run 45949654663.
        assert!(!package_output_ok(b"NEXUS_PACKAGE_ABSENT\nNEXUS_DNS_FALLBACK\nNEXUS_PACKAGE_INSTALLED\njq: A lightweight and flexible command-line JSON processor\njq: https://jqlang.github.io/jq/\njq: 387 KiB\njq-1.8.2\nNEXUS_JQ_OK\n"));
        assert!(!package_output_ok(b"NEXUS_PACKAGE_ABSENT\nNEXUS_PACKAGE_INSTALLED\nNEXUS_APK_VERSION=jq-1.8.2-r0\nNEXUS_BINARY_VERSION=jq-1.8.1\nNEXUS_JQ_OK\n"));
        assert!(!package_output_ok(b"NEXUS_PACKAGE_ABSENT\nNEXUS_PACKAGE_INSTALLED\nNEXUS_APK_VERSION=jq-1.8.2-r\nNEXUS_BINARY_VERSION=jq-1.8.2\nNEXUS_JQ_OK\n"));
    }
    #[test]
    fn capture_preserves_binary_and_streams() {
        let mut c=Capture::default();
        c.append(1,Stream::Stdout,b"a\xff").unwrap();
        c.append(2,Stream::Stderr,b"b").unwrap();
        c.append(3,Stream::Pty,b"c").unwrap();
        assert_eq!(c.stdout,b"a\xff");
        assert_eq!(c.stderr,b"b");
        assert_eq!(c.pty,b"c");
        assert_eq!(c.cursor,3);
    }
    #[test]
    fn capture_rejects_replay_and_oversize() {
        let mut c=Capture::default();
        c.append(1,Stream::Stdout,&vec![0;LIMIT]).unwrap();
        assert!(c.append(1,Stream::Stderr,b"").is_err());
        assert!(c.append(2,Stream::Pty,b"x").is_err());
        assert_eq!(c.cursor,1);
    }
}
