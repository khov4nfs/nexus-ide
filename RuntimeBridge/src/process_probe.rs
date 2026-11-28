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
    rolling_pty: bool,
    truncated_pty: bool,
    pty_total: u64,
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
        if self.rolling_pty && matches!(stream, Stream::Pty) {
            let skip = data.len().saturating_sub(LIMIT);
            let tail = &data[skip..];
            let remove = (self.pty.len()+tail.len()).saturating_sub(LIMIT);
            if skip > 0 || remove > 0 { self.truncated_pty = true; }
            self.pty.drain(..remove);
            self.pty.extend_from_slice(tail);
            self.pty_total = self.pty_total.checked_add(data.len() as u64).ok_or("PTY cursor overflow")?;
            self.cursor = seq;
            return Ok(());
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
// Official AGY terminal sign-in, in the same disposable guest as ACP.
// Never return raw PTY output, authorization code, stderr or token files.
pub(crate) async fn google_login(guest: &IshInstance, env: &HashMap<String,String>, fixture: bool) -> Result<String,String> {
    use crate::persistent_acp::{login_publish, login_take, LoginControl};
    let mut opts = SpawnOpts::cmd(["/opt/nexus/agy","acp","--login","--disable-telemetry"]);
    #[cfg(test)]
    if fixture { opts = shell(include_str!("../fixtures/google-login.sh"),env); }
    #[cfg(not(test))]
    if fixture { return Err("Fixture unavailable".into()); }
    opts.cwd = Some(Path::new("/tmp").to_path_buf());
    opts.envp = Some(env.iter().map(|(k,v)|(k.clone(),v.clone())).collect());
    opts.tty = true;
    opts.pipe_stdin = false;
    opts.size = Some(PtySize{cols:240,rows:40});
    let session = guest.spawn(opts).map_err(|_| "Login process spawn failed")?;
    let deadline = Instant::now()+Duration::from_secs(180);
    let mut capture = Capture::default();
    let result = async {
        let mut offered = false;
        let mut submitted: Option<String> = None;
        loop {
            capture.poll(&session,deadline).await?;
            if capture.closed {
                return if capture.exited && capture.code == Some(0) && capture.signal.unwrap_or(0)==0 {
                    submitted.ok_or("Login exited without code".into())
                } else { Err("Login process failed".into()) };
            }
            if !offered && !capture.exited {
                // Accept only complete, whitespace-terminated official Google
                // OAuth URLs; do not export arbitrary agent-provided output.
                if let Some(url) = google_oauth_url(&capture.pty) {
                    login_publish(url)?;
                    offered = true;
                }
            }
            match login_take()? {
                LoginControl::Cancel => return Err("Login cancelled".into()),
                LoginControl::Code(code) if offered && submitted.is_none() && !capture.exited => {
                    write_ready(&session,format!("{code}\n").as_bytes(),deadline).await?;
                    submitted = Some(code);
                }
                LoginControl::None => {}
                _ => return Err("Unexpected login input".into()),
            }
        }
    }.await;
    if result.is_err() {
        let _ = session.terminate().await;
        let _ = drain(&session,&mut capture,Instant::now()+TIMEOUT).await;
    }
    result
}

fn google_oauth_url(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut start = 0;
    // A word must end in whitespace; partial final frames are never offered.
    for (end, ch) in text.char_indices() {
        if !ch.is_ascii_whitespace() { continue; }
        let word = &text[start..end];
        start = end + ch.len_utf8();
        if word.len() > 4096 || !word.bytes().all(|b| b.is_ascii_graphic()) ||
            word.bytes().any(|b| matches!(b,b'\\'|b'<'|b'>'|b'"'|b'\''|b'\x60'|b'#')) { continue; }
        if (word.starts_with("https://accounts.google.com/o/oauth2/v2/auth?") ||
            word.starts_with("https://accounts.google.com/o/oauth2/auth?")) &&
            word.contains("client_id=") && word.contains("state=") {
            return Some(word.to_owned());
        }
    }
    None
}

#[cfg(test)]
#[test]
fn google_oauth_url_only_complete_official_endpoint() {
    let url = "https://accounts.google.com/o/oauth2/v2/auth?client_id=fixture&state=fixture";
    assert_eq!(google_oauth_url(format!("{url}\r\n").as_bytes()).as_deref(),Some(url));
    assert!(google_oauth_url(url.as_bytes()).is_none());
    assert!(google_oauth_url(b"https://accounts.google.com.evil/o/oauth2/v2/auth?client_id=x&state=x\n").is_none());
    assert!(google_oauth_url(b"https://accounts.google.com@evil/o/oauth2/v2/auth?client_id=x&state=x\n").is_none());
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
# Fresh guest: create only the known provider config. Reopened guest:
# accept identical config, never overwrite changed files or follow symlinks.
base=/root/.gemini
dir="$base/antigravity-cli"
settings="$dir/settings.json"
test ! -L /root
test ! -L "$base"
test ! -L "$dir"
test ! -L "$settings"
if [ -e "$base" ]; then
    test -d "$base"
    test -d "$dir"
    test -f "$settings"
    test "$(wc -c < "$settings")" -le 8192
    expected=$(printf '%s\n' '{"modelProvider":"gemini"}' | /usr/bin/sha256sum)
    actual=$(/usr/bin/sha256sum "$settings")
    test "${expected%% *}" = "${actual%% *}"
else
    mkdir -p "$dir"
    (set -C; printf '%s\n' '{"modelProvider":"gemini"}' > "$settings")
fi
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
    guest_file_digest(guest,env,"/opt/nexus/agy",expected).await
}
pub(crate) async fn guest_file_digest(guest:&IshInstance,env:&HashMap<String,String>,path:&str,expected:&str)->Value {
    let name = "agy_binary_digest";
    let mut opts = SpawnOpts::cmd(["/usr/bin/sha256sum", path]);
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
        && capture.stdout == format!("{expected}  {path}\n").as_bytes();
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

// Only the persistent transport can advertise virtual file and restricted terminal grants.
pub(crate) fn terminal_initialize_request(bytes:&[u8])->bool {
    if bytes.len()>4096 || bytes.last()!=Some(&b'\n') || bytes[..bytes.len()-1].contains(&b'\n') {return false;}
    let Ok(mut frame)=serde_json::from_slice::<Value>(bytes) else {return false;};
    let Some(caps)=frame["params"]["clientCapabilities"].as_object_mut() else {return false;};
    if caps.remove("terminal")!=Some(json!(true)) {return false;}
    let wire=format!("{frame}\n");
    valid_initialize_request(wire.as_bytes()) || snapshot_initialize_request(wire.as_bytes()) || edit_initialize_request(wire.as_bytes())
}
pub(crate) fn snapshot_initialize_request(bytes:&[u8])->bool { snapshot_request(bytes,false) }
pub(crate) fn edit_initialize_request(bytes:&[u8])->bool { snapshot_request(bytes,true) }
fn snapshot_request(bytes:&[u8],write:bool)->bool {
    if bytes.len()>4096 || bytes.last()!=Some(&b'\n') || bytes[..bytes.len()-1].contains(&b'\n') {return false;}
    let Ok(frame)=serde_json::from_slice::<Value>(bytes) else {return false;};
    let mut expected:Value=serde_json::from_slice(AGY_INITIALIZE_REQUEST).expect("fixed request");
    expected["params"]["clientCapabilities"]=json!({"fs":{"readTextFile":true,"writeTextFile":write}});
    if let Some(caps)=frame["params"]["clientCapabilities"].as_object() {
        if caps.get("terminal")==Some(&json!(true)) {
            expected["params"]["clientCapabilities"]["terminal"]=json!(true);
        }
    }
    frame==expected
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

    pub(crate) async fn notify(&self, bytes: &[u8]) -> Result<(),String> {
        write_ready(&self.session, bytes, Instant::now() + TIMEOUT).await
    }

    pub(crate) async fn request(&mut self, bytes: &[u8], id: i64) -> Result<Value,String> {
        self.request_updates(bytes, id, |_| Ok(())).await
    }

    pub(crate) async fn request_updates<F>(&mut self, bytes: &[u8], id: i64, update: F) -> Result<Value,String>
    where F: FnMut(&Value) -> Result<(),String> {
        self.request_controlled(bytes, id, update, || None, |_,_| Err("Unexpected agent request".into()), None).await
    }

    // Control is checked by the owner of the same stdin stream, never by
    // another writer. Cancellation is a notification, not a new request ID.
    pub(crate) async fn request_controlled<F, C, R>(&mut self, bytes: &[u8], id: i64, mut update: F, mut control: C, mut server_request: R, mut terminal: Option<&mut TerminalGrant<'_>>) -> Result<Value,String>
    where F: FnMut(&Value) -> Result<(),String>, C: FnMut() -> Option<Vec<u8>>, R: FnMut(&Value, Instant) -> Result<Value,String> {
        let mut deadline = Instant::now() + Duration::from_secs(90);
        let mut server_ids=std::collections::HashSet::new();
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
                if frame.get("method").is_some() && frame.get("id").is_some() &&
                    frame.get("result").is_none() && frame.get("error").is_none() && answer.is_none() {
                    let request_id=frame.get("id").filter(|v|v.as_i64().is_some() || v.as_str().is_some_and(|s|!s.is_empty()&&s.len()<=256)).ok_or("Invalid server request ID")?;
                    if server_ids.len()>=16 || !server_ids.insert(request_id.to_string()) {return Err("Duplicate or excessive server request".into());}
                    let reply = if frame["method"].as_str().is_some_and(|m|m.starts_with("terminal/")) {
                        match terminal.as_deref_mut() {
                            Some(t)=>t.handle(&frame,deadline).await?,
                            None=>json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32602,"message":"No terminal grant"}}),
                        }
                    } else {server_request(&frame,deadline)?};
                    let wire = format!("{reply}\n");
                    write_ready(&self.session, wire.as_bytes(), deadline).await?;
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
                // All complete frames have been consumed. Retain monotonic transport
                // cursor, not the previous request payloads. Per-request limit unchanged.
                self.capture.stdout.clear();
                self.capture.stderr.clear();
                self.offset=0;
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


// One explicitly approved argv per prompt. The bridge never interpolates a
// shell command or accepts agent-selected arguments, cwd or environment.
pub(crate) fn terminal_command(choice:&str)->Option<Vec<String>> {
    if choice=="uname" {return Some(vec!["/usr/bin/uname".into(),"-m".into()]);}
    if choice=="pwd" {return Some(vec!["/usr/bin/pwd".into()]);}
    if choice.len()>2048 {return None;}
    let argv:Vec<String>=serde_json::from_str(choice).ok()?;
    let program=argv.first()?;
    if argv.len()>17 || program.len()>256 || !program.starts_with('/') ||
        program.split('/').skip(1).any(|s|s.is_empty()||s=="."||s==".."||!s.bytes().all(|b|b.is_ascii_alphanumeric()||b"._+-".contains(&b))) ||
        argv.iter().any(|s|s.len()>512||s.chars().any(char::is_control)) {return None;}
    Some(argv)
}
struct GrantedProcess {
    session: std::sync::Arc<IshSession>,
    capture: Capture,
    limit: usize,
}
pub(crate) struct TerminalGrant<'a> {
    guest: &'a IshInstance,
    session_id: &'a str,
    terminal_id: String,
    choice: Option<&'a str>,
    key: &'a str,
    used: bool,
    process: Option<GrantedProcess>,
}
impl<'a> TerminalGrant<'a> {
    pub(crate) fn new(guest:&'a IshInstance,session_id:&'a str,response_id:i64,choice:Option<&'a str>,key:&'a str)->Self {
        Self {guest,session_id,terminal_id:format!("nexus-term-{response_id}"),choice,key,used:false,process:None}
    }
    pub(crate) fn used(&self)->bool {self.used}
    async fn stop(process:&mut GrantedProcess,deadline:Instant)->Result<(),String> {
        if !process.capture.closed {
            if !process.capture.exited {process.session.terminate().await.map_err(|_|"Terminal termination failed")?;}
            drain(&process.session,&mut process.capture,deadline).await?;
        }
        Ok(())
    }
    pub(crate) async fn cleanup(&mut self)->Result<(),String> {
        if let Some(mut p)=self.process.take() {Self::stop(&mut p,Instant::now()+TIMEOUT).await?;}
        Ok(())
    }
    pub(crate) async fn handle(&mut self,frame:&Value,deadline:Instant)->Result<Value,String> {
        let id=&frame["id"];
        let params=&frame["params"];
        let deny=||json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"No matching terminal grant"}});
        if params["sessionId"]!=self.session_id || self.choice.is_none() ||
            crate::persistent_acp::prompt_cancel_requested() {return Ok(deny());}
        let method=frame["method"].as_str().unwrap_or("");
        if method=="terminal/create" {
            if self.used {return Ok(deny());}
            let Some(argv)=self.choice.and_then(terminal_command) else {return Ok(deny());};
            if argv.iter().any(|arg|arg.contains(self.key)) {return Ok(deny());}
            let program=argv[0].as_str();
            let args=&argv[1..];
            let requested=params["command"].as_str();
            // Keep basename aliases only for the two original fixed presets.
            let legacy=matches!(self.choice,Some("uname"|"pwd"));
            let name=program.rsplit('/').next().unwrap_or(program);
            if requested!=Some(program) && !(legacy && requested==Some(name)) {return Ok(deny());}
            let args_ok=match params.get("args") {
                None=>args.is_empty(),
                Some(v)=>v.as_array().is_some_and(|a|a.len()==args.len() && a.iter().zip(args).all(|(v,s)|v.as_str()==Some(s.as_str()))),
            };
            if !args_ok || !params.get("env").is_none_or(|v|v.as_array().is_some_and(|a|a.is_empty())) ||
                !params.get("cwd").is_none_or(|v|v.is_null()||v.as_str()==Some("/tmp")) {return Ok(deny());}
            let limit=match params.get("outputByteLimit") {
                None|Some(Value::Null)=>4096,
                Some(v)=>match v.as_u64(){Some(n) if n>0=>n.min(4096) as usize,_=>return Ok(deny())}
            };
            let mut opts=SpawnOpts::cmd(argv.iter().map(String::as_str).collect::<Vec<_>>());
            opts.cwd=Some(Path::new("/tmp").to_path_buf());
            opts.envp=Some(vec![("PATH".into(),"/usr/bin:/bin".into()),("LANG".into(),"C".into()),("HOME".into(),"/tmp".into())]);
            opts.tty=true;
            opts.pipe_stdin=false;
            opts.size=Some(PtySize{cols:80,rows:24});
            self.used=true;
            let session=self.guest.spawn(opts).map_err(|_|"Terminal spawn failed")?;
            self.process=Some(GrantedProcess{session,capture:Capture{rolling_pty:true,..Default::default()},limit});
            return Ok(json!({"jsonrpc":"2.0","id":id,"result":{"terminalId":self.terminal_id}}));
        }
        if params["terminalId"]!=self.terminal_id {return Ok(deny());}
        let Some(p)=self.process.as_mut() else {return Ok(deny());};
        let bounded=deadline.min(Instant::now()+TIMEOUT);
        let result=match method {
            "terminal/output"=>{
                if !p.capture.closed {p.capture.poll(&p.session,bounded).await?;}
                // Withhold partial command output until closed.
                // Do not export credentials, control sequences or arbitrary raw bytes.
                let mut output=String::new();
                let mut truncated=false;
                if p.capture.closed {
                    let text=String::from_utf8_lossy(&p.capture.pty);
                    output=text.chars().filter(|c|!c.is_control() || *c=='\n' || *c=='\t').collect();
                    if self.key.is_empty() || text.contains(self.key) || output.contains(self.key) {return Err("Terminal credential output blocked".into());}
                    truncated=p.capture.truncated_pty || output.len()>p.limit;
                    let mut start=output.len().saturating_sub(p.limit);
                    while !output.is_char_boundary(start) {start+=1;}
                    output=output[start..].to_owned();
                }
                let mut r=json!({"output":output,"truncated":truncated});
                if p.capture.closed {r["exitStatus"]=terminal_status(&p.capture);}
                r
            }
            "terminal/wait_for_exit"=>{
                while !p.capture.closed {
                    if crate::persistent_acp::prompt_cancel_requested() {Self::stop(p,bounded).await?;break;}
                    p.capture.poll(&p.session,bounded).await?;
                }
                terminal_status(&p.capture)
            }
            "terminal/kill"=>{Self::stop(p,bounded).await?;json!({})}
            "terminal/release"=>{Self::stop(p,bounded).await?;self.process=None;json!({})}
            _=>return Ok(deny()),
        };
        Ok(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}
fn terminal_status(c:&Capture)->Value {
    let signal=c.signal.filter(|s|*s!=0).map(|s|match s {2=>"SIGINT".to_owned(),9=>"SIGKILL".to_owned(),15=>"SIGTERM".to_owned(),_=>format!("SIG{s}")});
    json!({"exitCode":c.code,"signal":signal})
}
#[cfg(test)]
#[test]
fn terminal_command_scope() {
    assert_eq!(terminal_command("uname"),Some(vec!["/usr/bin/uname".to_owned(),"-m".to_owned()]));
    assert_eq!(terminal_command("pwd"),Some(vec!["/usr/bin/pwd".to_owned()]));
    assert_eq!(terminal_command(r#"["/usr/bin/printf","%s","hello world"]"#).unwrap()[2],"hello world");
    assert!(terminal_command(r#"["/usr/bin/../bin/sh"]"#).is_none());
    assert!(terminal_command(r#"["printf","hello"]"#).is_none());
    assert!(terminal_command(r#"["/usr/bin/printf","\u0000"]"#).is_none());
    assert!(terminal_command("sh").is_none());
    assert!(terminal_command("uname; cat /etc/passwd").is_none());
}

// Run only an explicitly reviewed local lifecycle hook, never an agent string.
pub(crate) async fn local_hook(guest:&IshInstance,env:&HashMap<String,String>,command:&str)->bool {
    local_hook_with_timeout(guest,env,command,30).await
}
pub(crate) async fn local_hook_with_timeout(guest:&IshInstance,env:&HashMap<String,String>,command:&str,seconds:u64)->bool {
    run_hook(guest,env,command,seconds,crate::local_jobs::stop_requested).await
}
pub(crate) async fn acp_turn_hook(guest:&IshInstance,env:&HashMap<String,String>,command:&str)->bool{
    run_hook(guest,env,command,30,crate::persistent_acp::prompt_cancel_requested).await
}
async fn run_hook(guest:&IshInstance,env:&HashMap<String,String>,command:&str,seconds:u64,cancel:fn()->bool)->bool{
    if cancel(){return false;}
    let mut opts=shell(command,env);
    opts.cwd=Some(Path::new("/tmp").to_path_buf());
    opts.pipe_stdin=false;
    let Ok(session)=guest.spawn(opts) else {return false;};
    let mut capture=Capture::default();
    let deadline=Instant::now()+Duration::from_secs(seconds);
    let result:Result<(),String>=async {
        while !capture.closed {
            if cancel() {return Err("Hook cancelled".into());}
            capture.poll(&session,deadline).await?;
        }
        Ok(())
    }.await;
    if result.is_err() {
        let _=session.terminate().await;
        let _=drain(&session,&mut capture,Instant::now()+TIMEOUT).await;
    }
    let ok=result.is_ok() && capture.closed && capture.exited && capture.code==Some(0) && capture.signal.unwrap_or(0)==0 && !cancel();
    #[cfg(test)]
    if std::env::var_os("NEXUS_WORKBENCH_CI_TRACE").is_some(){
        // Only this synthetic CI workflow enables tracing. No user data.
        eprintln!("CI Workbench hook ok={ok}: {:?}; transport={:?}",capture.evidence(),result.err());
    }
    ok
}

// Codex app-server JSONL intentionally does not use ACP's JSON-RPC version
// requirement. Only the fixed initialize/account read sequence is exposed.
async fn codex_next(session:&IshSession,capture:&mut Capture,deadline:Instant)->Result<Value,String>{
    loop {
        if crate::local_jobs::stop_requested()||Instant::now()>=deadline{return Err("App-server cancelled or timed out".into());}
        if let Some(end)=capture.stdout.iter().position(|b|*b==b'\n'){
            let line=capture.stdout.drain(..=end).collect::<Vec<_>>();
            let frame:Value=serde_json::from_slice(&line).map_err(|_|"Invalid app-server frame")?;
            if !frame.is_object(){return Err("Invalid app-server message".into());}
            if frame.get("method").is_some() && frame.get("id").is_some(){return Err("Unexpected app-server request; no approval granted".into());}
            return Ok(frame);
        }
        if capture.exited||capture.closed||!capture.pty.is_empty(){return Err("App-server stopped".into());}
        capture.stderr.clear();
        capture.poll(session,deadline).await?;
    }
}
async fn codex_read(session:&IshSession,capture:&mut Capture,id:i64,deadline:Instant,early:&mut Vec<Value>)->Result<Value,String>{
    loop{
        let frame=codex_next(session,capture,deadline).await?;
        if frame["method"].is_string(){
            if early.len()>=64{return Err("Too many app-server notifications".into());}
            early.push(frame);continue;
        }
        if frame["id"].as_i64()!=Some(id)||frame.get("error").is_some()||!frame["result"].is_object(){return Err("App-server response rejected".into());}
        return Ok(frame["result"].clone());
    }
}
fn codex_id(v:&Value)->Result<&str,String>{v.as_str().filter(|s|!s.is_empty()&&s.len()<=256&&!s.chars().any(char::is_control)).ok_or("Invalid app-server identifier".into())}
fn codex_readonly(result:&Value)->bool{
    result["approvalPolicy"]=="never"&&result["sandbox"]["type"]=="readOnly"&&result["sandbox"]["networkAccess"]==false
}
async fn codex_open_thread(session:&IshSession,capture:&mut Capture,config:Option<&crate::local_jobs::NativeSession>,deadline:Instant)->Result<String,String>{
    let mut early=Vec::new();
    let start=if let Some(handle)=config.and_then(|c|c.resume.as_deref()){
        json!({"id":3,"method":"thread/resume","params":{"threadId":handle,"cwd":"/tmp","sandbox":"read-only","approvalPolicy":"never"}})
    }else{
        json!({"id":3,"method":"thread/start","params":{"cwd":"/tmp","ephemeral":!config.is_some_and(|c|c.retain),"sandbox":"read-only","approvalPolicy":"never"}})
    };
    write_ready(session,format!("{start}\n").as_bytes(),deadline).await?;
    let result=codex_read(session,capture,3,deadline,&mut early).await?;
    if !codex_readonly(&result){return Err("Effective thread policy not confirmed".into());}
    let thread=codex_id(&result["thread"]["id"])?.to_owned();
    if config.and_then(|c|c.resume.as_deref()).is_some_and(|h|h!=thread){return Err("Resumed thread mismatch".into());}
    Ok(thread)
}
async fn codex_text_turn(session:&IshSession,capture:&mut Capture,prompt:&str,deadline:Instant)->Result<String,String>{
    let thread=codex_open_thread(session,capture,None,deadline).await?;
    codex_turn(session,capture,&thread,4,prompt,deadline).await
}
async fn codex_turn(session:&IshSession,capture:&mut Capture,thread:&str,id:i64,prompt:&str,deadline:Instant)->Result<String,String>{
    let mut early=Vec::new();
    let start=json!({"id":id,"method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":prompt,"text_elements":[]}] }});
    write_ready(session,format!("{start}\n").as_bytes(),deadline).await?;
    let result=codex_read(session,capture,id,deadline,&mut early).await?;
    let turn=codex_id(&result["turn"]["id"])?.to_owned();
    let mut queue=std::collections::VecDeque::from(early);
    let mut output=String::new();
    for _ in 0..512{
        let frame=match queue.pop_front(){Some(f)=>f,None=>codex_next(session,capture,deadline).await?};
        let p=&frame["params"];
        let method=frame["method"].as_str().ok_or("Unexpected turn response")?;
        match method{
            "item/agentMessage/delta"=>{
                if p["threadId"]!=thread||p["turnId"]!=turn{return Err("Turn delta mismatch".into());}
                let text=p["delta"].as_str().ok_or("Invalid delta")?;
                if output.len()+text.len()>8192||text.chars().any(|c|c.is_control()&&c!='\n'&&c!='\t'){return Err("Unsupported turn output".into());}
                output.push_str(text);
            },
            "item/started"|"item/completed"=>{
                if p["threadId"]!=thread||p["turnId"]!=turn{return Err("Turn item mismatch".into());}
                if !matches!(p["item"]["type"].as_str(),Some("userMessage"|"agentMessage"|"reasoning"|"plan")){return Err("Non-text item observed; stopping turn".into());}
            },
            "turn/completed"=>{
                if p["threadId"]!=thread||p["turn"]["id"]!=turn||p["turn"]["status"]!="completed"||output.trim().is_empty(){return Err("Turn not successfully completed".into());}
                return Ok(output);
            },
            "turn/started"=>{
                if p["threadId"]!=thread||p["turn"]["id"]!=turn{return Err("Turn start mismatch".into());}
            },
            // Do not expose unrelated metadata or treat it as approval.
            _=>{}
        }
    }
    Err("Turn event limit exceeded".into())
}
pub(crate) async fn codex_app_server_probe(guest:&IshInstance,env:&HashMap<String,String>,fixture:bool,prompt:Option<&str>)->Value{
    codex_run(guest,env,fixture,prompt,None).await
}
pub(crate) async fn codex_session(guest:&IshInstance,env:&HashMap<String,String>,prompt:&str,config:&crate::local_jobs::NativeSession,fixture:bool)->Value{
    codex_run(guest,env,fixture,Some(prompt),Some(config)).await
}
async fn codex_run(guest:&IshInstance,env:&HashMap<String,String>,fixture:bool,prompt:Option<&str>,config:Option<&crate::local_jobs::NativeSession>)->Value{
    let mut opts=SpawnOpts::cmd(["/opt/nexus/codex","app-server"]);
    #[cfg(test)]
    if fixture{opts=shell(include_str!("../fixtures/codex-app-server.sh"),env);}
    #[cfg(not(test))]
    if fixture{return json!({"ok":false});}
    opts.cwd=Some(Path::new("/tmp").to_path_buf());
    opts.envp=Some(env.iter().map(|(k,v)|(k.clone(),v.clone())).collect());
    opts.pipe_stdin=true;
    let Ok(session)=guest.spawn(opts) else{return json!({"ok":false,"error":"App-server spawn failed"});};
    let mut capture=Capture::default();
    let deadline=Instant::now()+Duration::from_secs(if prompt.is_some(){120}else{30});
    let result:Result<Value,String>=async{
        let init=json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"nexus-ios","version":"0.6.0"}}});
        let mut early=Vec::new();
        write_ready(&session,format!("{init}\n").as_bytes(),deadline).await?;
        let init=codex_read(&session,&mut capture,1,deadline,&mut early).await?;
        if !init["userAgent"].as_str().is_some_and(|s|!s.is_empty()){return Err("Unsupported initialize response".into());}
        early.clear();
        write_ready(&session,b"{\"method\":\"initialized\"}\n",deadline).await?;
        write_ready(&session,b"{\"id\":2,\"method\":\"account/read\",\"params\":{\"refreshToken\":false}}\n",deadline).await?;
        let account=codex_read(&session,&mut capture,2,deadline,&mut early).await?;
        let requires=account["requiresOpenaiAuth"].as_bool().ok_or("Unsupported account response")?;
        let present=match account.get("account"){Some(Value::Null)=>false,Some(Value::Object(_))=>true,_=>return Err("Unsupported account shape".into())};
        let mut r=json!({"ok":true,"initializeVerified":true,"accountReadVerified":true,"accountPresent":present,"requiresOpenaiAuth":requires});
        if let Some(prompt)=prompt{
            if !present{return Err("CLI sign-in required before native chat".into());}
            if let Some(config)=config{
                use crate::local_jobs::{agent_complete,agent_next,AgentNext,valid_handle};
                let thread=codex_open_thread(&session,&mut capture,Some(config),deadline).await?;
                if config.retain&&!valid_handle(&thread){return Err("Invalid saved thread handle".into());}
                let mut message=prompt.to_owned();
                let mut completed=0i64;
                loop{
                    let text=codex_turn(&session,&mut capture,&thread,4+completed,&message,Instant::now()+Duration::from_secs(120)).await?;
                    if !agent_complete(&message,&text,if config.retain{Some(thread.as_str())}else{None}){return Err("Session cancelled".into());}
                    completed+=1;
                    match agent_next().await{
                        AgentNext::Prompt(next)=>message=next,
                        AgentNext::Finish=>break,
                        AgentNext::Stop=>return Err("Session stopped".into()),
                    }
                }
                r["turnsCompleted"]=json!(completed);
                r["sessionFinished"]=json!(true);
            }else{
                let text=codex_text_turn(&session,&mut capture,prompt,deadline).await?;
                r["turnCompleted"]=json!(true);
                r["response"]=json!(text);
            }
        }
        Ok(r)
    }.await;
    let stopped=session.terminate().await.is_ok();
    let closed=drain(&session,&mut capture,Instant::now()+TIMEOUT).await.is_ok()&&capture.closed;
    match result{
        Ok(mut r)=>{r["ok"]=json!(stopped&&closed);r["transportClosed"]=json!(closed);if !stopped||!closed{r.as_object_mut().unwrap().remove("response");}r},
        Err(_)=>json!({"ok":false,"error":"App-server operation rejected, sign-in needed, timed out or cancelled","transportClosed":closed}),
    }
}

// Official Claude CLI JSON mode. Never parse or export account identity,
// arbitrary stderr or auth tokens; the harness owns its authentication.
async fn claude_json(guest:&IshInstance,env:&HashMap<String,String>,argv:Vec<&str>,fixture:bool,seconds:u64)->Result<(Value,i32),String>{
    let mut opts=SpawnOpts::cmd(argv.iter().copied());
    #[cfg(test)]
    if fixture{
        let mut args=vec!["/bin/sh","-c",include_str!("../fixtures/claude-json.sh"),"nexus-fixture"];
        args.extend(argv.iter().skip(1).copied());
        opts=SpawnOpts::cmd(args);
    }
    #[cfg(not(test))]
    if fixture{return Err("Fixture unavailable".into());}
    opts.cwd=Some(Path::new("/tmp").to_path_buf());
    opts.envp=Some(env.iter().map(|(k,v)|(k.clone(),v.clone())).collect());
    opts.pipe_stdin=false;
    let session=guest.spawn(opts).map_err(|_|"Claude spawn failed")?;
    let mut capture=Capture::default();
    let deadline=Instant::now()+Duration::from_secs(seconds);
    let result:Result<(),String>=async{
        while !capture.closed {
            if crate::local_jobs::stop_requested(){return Err("Cancelled".into());}
            capture.poll(&session,deadline).await?;
        }
        Ok(())
    }.await;
    if result.is_err(){let _=session.terminate().await;let _=drain(&session,&mut capture,Instant::now()+TIMEOUT).await;return Err("Claude operation cancelled or timed out".into());}
    if !capture.exited||capture.signal.unwrap_or(0)!=0||!capture.pty.is_empty(){return Err("Claude process did not exit normally".into());}
    let frame:Value=serde_json::from_slice(&capture.stdout).map_err(|_|"Invalid Claude JSON")?;
    if !frame.is_object(){return Err("Invalid Claude result".into());}
    Ok((frame,capture.code.ok_or("No exit code")?))
}
pub(crate) async fn claude_auth_probe(guest:&IshInstance,env:&HashMap<String,String>,fixture:bool)->Value{
    match claude_json(guest,env,vec!["/opt/nexus/claude","auth","status"],fixture,30).await{
        Ok((frame,code)) if matches!(code,0|1)=>{
            let auth=frame["authMethod"].as_str().unwrap_or("");
            let valid=matches!(auth,"none"|"claude.ai"|"oauth_token"|"api_key"|"api_key_helper"|"third_party");
            let logged=code==0;
            if valid && (logged==(auth!="none")) && frame.get("loggedIn").is_none_or(|v|v.as_bool()==Some(logged)){
                json!({"ok":true,"accountReadVerified":true,"accountPresent":logged})
            }else{json!({"ok":false,"error":"Unsupported Claude authentication response"})}
        },
        _=>json!({"ok":false,"error":"Claude authentication status unavailable"}),
    }
}
pub(crate) async fn claude_text_turn(guest:&IshInstance,env:&HashMap<String,String>,prompt:&str,fixture:bool)->Value{
    if !fixture && claude_auth_probe(guest,env,false).await["accountPresent"]!=true{return json!({"ok":false,"error":"Use official CLI sign-in first"});}
    // No shell interpolation, no custom hooks/plugins, no bypass permissions.
    let argv=vec!["/opt/nexus/claude","--safe-mode","--print","--output-format","json","--tools","","--disallowedTools","mcp__*","--strict-mcp-config","--mcp-config","{\"mcpServers\":{}}","--permission-mode","dontAsk","--no-session-persistence","--",prompt];
    match claude_json(guest,env,argv,fixture,120).await{
        Ok((frame,0)) if frame["type"]=="result"&&frame["subtype"]=="success"&&frame["is_error"]==false=>{
            let Some(text)=frame["result"].as_str().filter(|s|!s.trim().is_empty()&&s.len()<=8192&&s.chars().all(|c|!c.is_control()||c=='\n'||c=='\t')) else{return json!({"ok":false,"error":"Invalid Claude text result"});};
            json!({"ok":true,"turnCompleted":true,"response":text})
        },
        _=>json!({"ok":false,"error":"Claude text turn rejected, timed out or failed"}),
    }
}

pub(crate) async fn claude_session(guest:&IshInstance,env:&HashMap<String,String>,prompt:&str,config:&crate::local_jobs::NativeSession,fixture:bool)->Value{
    use crate::local_jobs::{agent_complete,agent_next,AgentNext,valid_uuid};
    if !fixture&&claude_auth_probe(guest,env,false).await["accountPresent"]!=true{return json!({"ok":false,"error":"Use official CLI sign-in first"});}
    let mut handle=config.resume.clone();
    let mut message=prompt.to_owned();
    let mut completed=0;
    loop{
        let mut argv=vec!["/opt/nexus/claude","--safe-mode","--print","--output-format","json","--tools","","--disallowedTools","mcp__*","--strict-mcp-config","--mcp-config","{\"mcpServers\":{}}","--permission-mode","dontAsk","--max-turns","1","--no-chrome"];
        if let Some(id)=handle.as_deref(){argv.extend(["--resume",id]);}
        argv.extend(["--",message.as_str()]);
        let Ok((frame,0))=claude_json(guest,env,argv,fixture,120).await else{return json!({"ok":false,"error":"Claude session failed or cancelled"});};
        if frame["type"]!="result"||frame["subtype"]!="success"||frame["is_error"]!=false{return json!({"ok":false,"error":"Claude turn rejected"});}
        let Some(id)=frame["session_id"].as_str().filter(|id|valid_uuid(id))else{return json!({"ok":false,"error":"Missing valid Claude session ID"});};
        if handle.as_deref().is_some_and(|h|h!=id){return json!({"ok":false,"error":"Claude session mismatch"});}
        let Some(text)=frame["result"].as_str().filter(|s|!s.trim().is_empty()&&s.len()<=8192&&s.chars().all(|c|!c.is_control()||c=='\n'||c=='\t'))else{return json!({"ok":false,"error":"Invalid Claude text result"});};
        handle=Some(id.to_owned());
        if !agent_complete(&message,text,if config.retain{handle.as_deref()}else{None}){return json!({"ok":false,"error":"Session stopped"});}
        completed+=1;
        match agent_next().await{
            AgentNext::Prompt(next)=>message=next,
            AgentNext::Finish=>return json!({"ok":true,"turnsCompleted":completed,"sessionFinished":true}),
            AgentNext::Stop=>return json!({"ok":false,"error":"Session stopped"}),
        }
    }
}

// Fixed guest folder, direct argv, create-only export. No host filesystem mount.
pub(crate) async fn workspace_transfer(guest:&IshInstance,env:&HashMap<String,String>,request:&crate::local_jobs::TransferInput)->Value{
    use base64::{engine::general_purpose::STANDARD,Engine as _};
    let script=r#"
set -eu
umask 077
dir=/root/nexus-transfer
test ! -L /root
test ! -L "$dir"
mkdir -p "$dir"
test -d "$dir"
rel=$1
while [ "$rel" != "${rel#*/}" ]; do
    part=${rel%%/*}
    rel=${rel#*/}
    dir="$dir/$part"
    test ! -L "$dir"
    if [ "$2" = push ]; then mkdir -p "$dir"; fi
    test -d "$dir"
done
target="$dir/$rel"
test ! -L "$target"
if [ "$2" = push ]; then
    test ! -e "$target"
    tmp=$(mktemp "$dir/.incoming-XXXXXXXX")
    trap 'rm -f "$tmp"' EXIT
    printf 'NEXUS_TRANSFER_READY\n' >&2
    IFS= read -r wire
    printf '%s' "$wire" | /usr/bin/base64 -d > "$tmp"
    test "$(wc -c < "$tmp")" -le 8192
    ln -T "$tmp" "$target"
else
    test -f "$target"
    test "$(wc -c < "$target")" -le 8192
fi
head -c 8193 "$target" | /usr/bin/base64 -w0
"#;
    let mut opts=SpawnOpts::cmd(["/bin/sh","-c",script,"nexus-transfer",request.name.as_str(),if request.text.is_some(){"push"}else{"pull"}]);
    opts.cwd=Some(Path::new("/").to_path_buf());
    opts.envp=Some(env.iter().map(|(k,v)|(k.clone(),v.clone())).collect());
    opts.pipe_stdin=true;
    let Ok(session)=guest.spawn(opts)else{return json!({"ok":false,"error":"Transfer process spawn failed"});};
    let mut capture=Capture::default();
    let deadline=Instant::now()+Duration::from_secs(30);
    let result:Result<Value,String>=async{
        if crate::local_jobs::stop_requested(){return Err("Cancelled".into());}
        if let Some(text)=request.text.as_deref(){
            wait_ready(&session,&mut capture,deadline,|c|c.stderr==b"NEXUS_TRANSFER_READY\n").await?;
            if crate::local_jobs::stop_requested(){return Err("Cancelled".into());}
            write_ready(&session,format!("{}\n",STANDARD.encode(text.as_bytes())).as_bytes(),deadline).await?;
        }
        drain(&session,&mut capture,deadline).await?;
        if crate::local_jobs::stop_requested()||!capture.closed||capture.code!=Some(0)||capture.signal.unwrap_or(0)!=0||!capture.pty.is_empty(){return Err("Transfer rejected".into());}
        let bytes=STANDARD.decode(&capture.stdout).map_err(|_|"Invalid transfer encoding")?;
        let text=String::from_utf8(bytes).map_err(|_|"Invalid UTF-8")?;
        if text.len()>8192||text.contains('\0'){return Err("Invalid transfer text".into());}
        if let Some(original)=request.text.as_deref(){
            if text!=original{return Err("Export byte verification failed".into());}
            Ok(json!({"ok":true,"action":"push","name":request.name,"bytes":text.len(),"createOnly":true}))
        }else{
            Ok(json!({"ok":true,"action":"pull","name":request.name,"bytes":text.len(),"text":text}))
        }
    }.await;
    match result{
        Ok(r)=>r,
        Err(_)=>{let _=session.terminate().await;let _=drain(&session,&mut capture,Instant::now()+TIMEOUT).await;json!({"ok":false,"error":"Transfer rejected, cancelled, or destination already exists"})}
    }
}

// LSP 3.17 Content-Length framing; no workspace edits, paths or server requests.
fn lsp_frame(buffer:&mut Vec<u8>)->Result<Option<Value>,String>{
    let Some(end)=buffer.windows(4).position(|b|b==b"\r\n\r\n")else{
        return if buffer.len()>1024{Err("LSP header too large".into())}else{Ok(None)};
    };
    if end>1024{return Err("LSP header too large".into());}
    let header=std::str::from_utf8(&buffer[..end]).map_err(|_|"Invalid LSP header")?;
    let mut length=None;
    for line in header.split("\r\n"){
        let (key,value)=line.split_once(':').ok_or("Invalid LSP header")?;
        if key.eq_ignore_ascii_case("Content-Length"){
            let value=value.trim();
            if length.is_some()||value.is_empty()||!value.bytes().all(|b|b.is_ascii_digit()){return Err("Invalid LSP length".into());}
            let n=value.parse::<usize>().map_err(|_|"Invalid LSP length")?;
            if !(1..=32768).contains(&n){return Err("LSP body limit".into());}
            length=Some(n);
        }else if !key.eq_ignore_ascii_case("Content-Type"){return Err("Unknown LSP header".into());}
    }
    let n=length.ok_or("Missing LSP length")?;
    let start=end+4;
    if buffer.len()<start+n{return Ok(None);}
    let frame:Value=serde_json::from_slice(&buffer[start..start+n]).map_err(|_|"Invalid LSP JSON")?;
    buffer.drain(..start+n);
    if !frame.is_object()||frame["jsonrpc"]!="2.0"{return Err("Invalid LSP RPC".into());}
    if frame.get("method").is_some()&&frame.get("id").is_some(){return Err("LSP server request denied".into());}
    Ok(Some(frame))
}
async fn lsp_send(session:&IshSession,value:Value,deadline:Instant)->Result<(),String>{
    let body=value.to_string();
    let wire=format!("Content-Length: {}\r\n\r\n{}",body.len(),body);
    write_ready(session,wire.as_bytes(),deadline).await
}
async fn lsp_next(session:&IshSession,capture:&mut Capture,deadline:Instant)->Result<Value,String>{
    loop{
        if crate::local_jobs::stop_requested()||Instant::now()>=deadline{return Err("LSP cancelled or timed out".into());}
        if let Some(f)=lsp_frame(&mut capture.stdout)?{return Ok(f);}
        if capture.exited||capture.closed||!capture.pty.is_empty(){return Err("LSP process stopped".into());}
        // Only CI's fixed synthetic document may produce server trace output.
        #[cfg(test)]
        if std::env::var_os("NEXUS_PYLSP_TRACE").is_some()&&!capture.stderr.is_empty(){
            eprintln!("pylsp stderr: {}",String::from_utf8_lossy(&capture.stderr));
        }
        capture.stderr.clear();
        capture.poll(session,deadline).await?;
    }
}
async fn lsp_reply(session:&IshSession,capture:&mut Capture,id:i64,deadline:Instant)->Result<Value,String>{
    for _ in 0..64{
        let f=lsp_next(session,capture,deadline).await?;
        if f.get("id").is_none()&&f["method"].is_string(){continue;}
        if f["id"].as_i64()!=Some(id)||f.get("error").is_some()||f.get("result").is_none(){return Err("LSP response rejected".into());}
        return Ok(f["result"].clone());
    }
    Err("LSP notification limit".into())
}
fn lsp_position(v:&Value,lines:&[&str])->Option<(usize,usize)>{
    let line=usize::try_from(v["line"].as_u64()?).ok()?;
    let character=usize::try_from(v["character"].as_u64()?).ok()?;
    let text=lines.get(line)?.trim_end_matches('\r');
    if !std::iter::once(0).chain(text.chars().scan(0usize,|n,c|{*n+=c.len_utf16();Some(*n)})).any(|n|n==character){return None;}
    Some((line,character))
}
fn lsp_project(params:&Value,request:&crate::local_jobs::LspInput)->Result<Vec<Value>,String>{
    lsp_project_version(params,&request.uri,&request.text,1,false)
}
fn lsp_project_version(params:&Value,uri:&str,text:&str,version:u32,strict:bool)->Result<Vec<Value>,String>{
    if params["uri"]!=uri||(strict&&params["version"].as_u64()!=Some(version as u64))||params.get("version").is_some_and(|v|!v.is_null()&&v.as_u64()!=Some(version as u64)){return Err("LSP diagnostic document mismatch".into());}
    let items=params["diagnostics"].as_array().filter(|a|a.len()<=64).ok_or("LSP diagnostics limit")?;
    let lines=text.split('\n').collect::<Vec<_>>();
    let mut result=Vec::new();
    let mut bytes=0;
    for item in items{
        let start=lsp_position(&item["range"]["start"],&lines).ok_or("Invalid LSP diagnostic range")?;
        let end=lsp_position(&item["range"]["end"],&lines).ok_or("Invalid LSP diagnostic range")?;
        if start>end{return Err("Reversed LSP range".into());}
        let message=item["message"].as_str().filter(|s|!s.is_empty()&&s.len()<=1024&&s.chars().all(|c|!c.is_control()||c=='\n'||c=='\t')).ok_or("Invalid LSP diagnostic message")?;
        bytes+=message.len();
        if bytes>8192{return Err("LSP text limit".into());}
        let severity=match item.get("severity"){None|Some(Value::Null)=>None,Some(v)=>Some(v.as_u64().filter(|n|(1..=4).contains(n)).ok_or("Invalid LSP severity")?)};
        result.push(json!({"range":{"start":{"line":start.0,"character":start.1},"end":{"line":end.0,"character":end.1}},"message":message,"severity":severity}));
    }
    Ok(result)
}
// Project only plain, single-range insertions. Never execute completion commands,
// resolve items, snippets, or apply additional edits.
fn lsp_completions(value:&Value,request:&crate::local_jobs::LspInput)->Result<Vec<Value>,String>{
    if value.is_null(){return Ok(Vec::new());}
    let items=if value.is_array(){value.as_array()}else{
        if value.get("itemDefaults").is_some(){return Err("Unsupported completion defaults".into());}
        value["items"].as_array()
    }.filter(|a|a.len()<=128).ok_or("Invalid completion list")?;
    let cursor=request.completion.ok_or("Missing completion position")?;
    let lines=request.text.split('\n').collect::<Vec<_>>();
    let mut result=Vec::new();
    let mut bytes=0;
    for item in items{
        if item.get("command").is_some()||item.get("additionalTextEdits").is_some()||item.get("textEditText").is_some()||
           item.get("insertTextFormat").is_some_and(|v|v.as_u64()!=Some(1))||
           item.get("insertTextMode").is_some_and(|v|v.as_u64()!=Some(1)){continue;}
        let Some(label)=item["label"].as_str().filter(|s|!s.is_empty()&&s.len()<=256&&!s.chars().any(char::is_control))else{continue;};
        let (start,end,text)=if let Some(edit)=item.get("textEdit"){
            let Some(start)=lsp_position(&edit["range"]["start"],&lines)else{continue;};
            let Some(end)=lsp_position(&edit["range"]["end"],&lines)else{continue;};
            if start.0!=cursor.0||end.0!=cursor.0||start>cursor||end<cursor||start>end{continue;}
            let Some(text)=edit["newText"].as_str()else{continue;};
            (start,end,text)
        }else{
            (cursor,cursor,item.get("insertText").and_then(Value::as_str).unwrap_or(label))
        };
        if text.is_empty()||text.len()>1024||text.chars().any(char::is_control){continue;}
        bytes+=label.len()+text.len();
        if result.len()>=32||bytes>8192{break;}
        result.push(json!({"label":label,"text":text,"range":{"start":{"line":start.0,"character":start.1},"end":{"line":end.0,"character":end.1}}}));
    }
    Ok(result)
}
// A single reviewed document and full-sync server. Versionless diagnostics are
// not safe to associate with subsequent editor contents; fail closed.
pub(crate) async fn live_lsp(guest:&IshInstance,env:&HashMap<String,String>,request:&crate::local_jobs::LspInput,fixture:bool)->Value{
    use crate::local_jobs::{lsp_publish,lsp_wait,LspNext};
    let mut opts=SpawnOpts::cmd(request.argv.iter().map(String::as_str));
    #[cfg(test)]
    if fixture{opts=shell(include_str!("../fixtures/lsp-live.sh"),env);}
    #[cfg(not(test))]
    if fixture{return json!({"ok":false});}
    opts.cwd=Some(Path::new("/tmp").to_path_buf());
    opts.envp=Some(env.iter().map(|(k,v)|(k.clone(),v.clone())).collect());
    opts.pipe_stdin=true;
    let Ok(session)=guest.spawn(opts)else{return json!({"ok":false,"error":"LSP spawn failed"});};
    let mut capture=Capture::default();
    let result:Result<u32,String>=async{
        let deadline=Instant::now()+Duration::from_secs(45);
        lsp_send(&session,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"workspaceFolders":null,"capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"publishDiagnostics":{"versionSupport":true},"synchronization":{"dynamicRegistration":false}}}}}),deadline).await?;
        let init=lsp_reply(&session,&mut capture,1,deadline).await?;
        let caps=init["capabilities"].as_object().ok_or("Missing LSP capabilities")?;
        let sync=&init["capabilities"]["textDocumentSync"];
        if caps.get("positionEncoding").is_some_and(|v|v!="utf-16")||!(sync.as_u64()==Some(1)||(sync["change"].as_u64()==Some(1)&&sync["openClose"]==true)){return Err("Full synchronization unsupported".into());}
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"initialized","params":{}}),deadline).await?;
        let mut text=request.text.clone();
        let mut version=1u32;
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":request.uri,"languageId":request.language,"version":version,"text":text}}}),deadline).await?;
        loop{
            let deadline=Instant::now()+Duration::from_secs(45);
            let mut found=None;
            for _ in 0..128{
                let frame=lsp_next(&session,&mut capture,deadline).await?;
                if frame.get("id").is_some(){return Err("Unexpected reply".into());}
                if frame["method"]=="textDocument/publishDiagnostics"{
                    let p=&frame["params"];
                    if p["uri"]==request.uri&&p["version"].as_u64().is_some_and(|v|v<version as u64){continue;}
                    found=Some(lsp_project_version(p,&request.uri,&text,version,true)?);break;
                }
            }
            if !lsp_publish(version,found.ok_or("No correlated diagnostics")?){return Err("LSP stopped".into());}
            match lsp_wait().await{
                LspNext::Change(next)=>{
                    version+=1;text=next;
                    lsp_send(&session,json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":request.uri,"version":version},"contentChanges":[{"text":text}]}}),Instant::now()+Duration::from_secs(45)).await?;
                }
                LspNext::Finish=>break,
                LspNext::Stop=>return Err("LSP stopped".into()),
            }
        }
        let deadline=Instant::now()+Duration::from_secs(30);
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":request.uri}}}),deadline).await?;
        lsp_send(&session,json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),deadline).await?;
        if !lsp_reply(&session,&mut capture,2,deadline).await?.is_null(){return Err("Invalid shutdown".into());}
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"exit"}),deadline).await?;
        drain(&session,&mut capture,deadline).await?;
        if !capture.closed||capture.code!=Some(0)||capture.signal.unwrap_or(0)!=0{return Err("LSP shutdown failed".into());}
        Ok(version)
    }.await;
    match result{
        Ok(n)=>json!({"ok":true,"versions":n,"transportClosed":true}),
        Err(error)=>{
            // Keep the production report generic; the fixture needs to prove
            // the exact rejection rather than pass on any unrelated failure.
            #[cfg(test)]
            let fixture_error=if fixture{Some(error.as_str())}else{None};
            #[cfg(not(test))]
            let fixture_error:Option<&str>=None;
            let _=session.terminate().await;
            let closed=drain(&session,&mut capture,Instant::now()+TIMEOUT).await.is_ok()&&capture.closed;
            let mut report=json!({"ok":false,"transportClosed":closed,"error":"Live LSP stopped, unsupported or invalid versioned diagnostics"});
            if let Some(reason)=fixture_error{report["fixtureError"]=json!(reason);}
            report
        }
    }
}

// CI-only: verify the real package inside the imported disposable ARM64 guest.
// APT runs on the Linux preparation runner, never in the app or shipped rootfs.
#[cfg(test)]
pub(crate) async fn verify_pylsp_for_ci(guest:&IshInstance,env:&HashMap<String,String>)->Result<String,String>{
    let script=r#"
set -eu
test "$(dpkg-query -W -f='${Version}' python3-pylsp)" = 1.10.0-1
test -x /usr/bin/pylsp
/usr/bin/python3 -c 'import pylsp, pyflakes; from importlib.metadata import version; assert version("python-lsp-server") == "1.10.0"'
printf '%s\n' NEXUS_PYLSP_1_10_0
"#;
    let mut opts=shell(script,env);opts.pipe_stdin=false;
    let session=guest.spawn(opts).map_err(|_|"Package verification process spawn failed")?;
    let mut capture=Capture::default();
    let result=drain(&session,&mut capture,Instant::now()+Duration::from_secs(60)).await;
    if result.is_err(){let _=session.terminate().await;let _=drain(&session,&mut capture,Instant::now()+TIMEOUT).await;}
    if result.is_err()||!capture.closed||capture.code!=Some(0)||capture.signal.unwrap_or(0)!=0||capture.stdout!=b"NEXUS_PYLSP_1_10_0\n"{
        // This guest has only CI packages and fixed synthetic source, no user credentials.
        eprintln!("CI pylsp package verification: {:?}",capture.evidence());
        return Err("Prepared Python LSP Server guest verification failed".into());
    }
    Ok("1.10.0-1".into())
}

// CI-only regression for glibc pthread_create/clone3 and real timer dispatch.
// Validate actual cross-thread execution, join, and repeated creation in guest.
#[cfg(test)]
pub(crate) async fn verify_python_threads_for_ci(guest:&IshInstance,env:&HashMap<String,String>)->bool{
    let script=r#"
set -eu
/usr/bin/python3 -u -c '
import threading
import queue
import time
# Python Event.wait uses a monotonic absolute futex deadline on glibc.
start = time.monotonic()
assert not threading.Event().wait(0.15), "unset event unexpectedly signaled"
elapsed = time.monotonic() - start
assert 0.12 <= elapsed <= 5, f"monotonic event timeout mismatch: {elapsed}"
for i in range(3):
    messages = queue.Queue()
    def worker():
        time.sleep(0.05)
        messages.put(("thread", threading.get_ident()))
    t = threading.Thread(target=worker)
    t.start()
    t.join(5)
    assert not t.is_alive(), "thread join timed out"
    kind, ident = messages.get_nowait()
    assert kind == "thread" and ident != threading.get_ident()
    started = time.monotonic()
    timer = threading.Timer(0.15, lambda: messages.put(("timer", threading.get_ident(), time.monotonic())))
    timer.start()
    timer.join(5)
    assert not timer.is_alive(), "timer join timed out"
    kind, ident, fired = messages.get_nowait()
    assert kind == "timer" and ident != threading.get_ident()
    assert 0.12 <= fired - started <= 5, f"timer deadline mismatch: {fired - started}"
print("NEXUS_PYTHON_THREADS_OK")
'
"#;
    let mut opts=shell(script,env);opts.pipe_stdin=false;
    let Ok(session)=guest.spawn(opts)else{return false;};
    let mut capture=Capture::default();
    let result=drain(&session,&mut capture,Instant::now()+Duration::from_secs(40)).await;
    if result.is_err(){let _=session.terminate().await;let _=drain(&session,&mut capture,Instant::now()+TIMEOUT).await;}
    let ok=result.is_ok()&&capture.closed&&capture.code==Some(0)&&capture.signal.unwrap_or(0)==0&&capture.stdout==b"NEXUS_PYTHON_THREADS_OK\n";
    if !ok{eprintln!("CI Python thread regression: {:?}",capture.evidence());}
    ok
}

pub(crate) async fn lsp_diagnostics(guest:&IshInstance,env:&HashMap<String,String>,request:&crate::local_jobs::LspInput,fixture:bool)->Value{
    let mut opts=SpawnOpts::cmd(request.argv.iter().map(String::as_str));
    #[cfg(test)]
    if fixture{opts=shell(include_str!("../fixtures/lsp-diagnostic.sh"),env);}
    #[cfg(not(test))]
    if fixture{return json!({"ok":false});}
    opts.cwd=Some(Path::new("/tmp").to_path_buf());
    opts.envp=Some(env.iter().map(|(k,v)|(k.clone(),v.clone())).collect());
    opts.pipe_stdin=true;
    let Ok(session)=guest.spawn(opts)else{return json!({"ok":false,"error":"LSP spawn failed"});};
    let mut capture=Capture::default();
    // Keep the production deadline bounded; allow extra cold-start time only
    // for the real-server integration test running under the interpreter.
    #[cfg(test)]
    let seconds=if std::env::var_os("NEXUS_PYLSP_TRACE").is_some(){120}else{45};
    #[cfg(not(test))]
    let seconds=45;
    let deadline=Instant::now()+Duration::from_secs(seconds);
    let mut stage="initialize";
    let result:Result<Value,String>=async{
        lsp_send(&session,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","workspaceFolders":null,"capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"publishDiagnostics":{"versionSupport":true},"completion":{"completionItem":{"snippetSupport":false}}}},"clientInfo":{"name":"nexus-ios","version":"0.8.0"}}}),deadline).await?;
        let init=lsp_reply(&session,&mut capture,1,deadline).await?;
        let caps=init["capabilities"].as_object().ok_or("Missing LSP capabilities")?;
        if caps.get("positionEncoding").is_some_and(|v|v!="utf-16"){return Err("Unsupported LSP position encoding".into());}
        stage="initialized";
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"initialized","params":{}}),deadline).await?;
        stage="didOpen";
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":request.uri,"languageId":request.language,"version":1,"text":request.text}}}),deadline).await?;
        stage="document response";
        let deadline=Instant::now()+Duration::from_secs(seconds);
        let (projected,shutdown_id)=if let Some((line,character))=request.completion{
            if !caps.get("completionProvider").is_some_and(Value::is_object){return Err("Server does not advertise completion".into());}
            lsp_send(&session,json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":request.uri},"position":{"line":line,"character":character},"context":{"triggerKind":1}}}),deadline).await?;
            let answer=lsp_reply(&session,&mut capture,2,deadline).await?;
            (json!({"completions":lsp_completions(&answer,request)?}),3)
        }else{
        let mut found=None;
        for _ in 0..128{
            let f=lsp_next(&session,&mut capture,deadline).await?;
            if f.get("id").is_some(){return Err("Unexpected LSP reply".into());}
            if f["method"]=="textDocument/publishDiagnostics"{found=Some(lsp_project(&f["params"],request)?);break;}
        }
        (json!({"diagnostics":found.ok_or("No push diagnostics received")?}),2)
        };
        stage="didClose and shutdown";
        let deadline=Instant::now()+Duration::from_secs(30);
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":request.uri}}}),deadline).await?;
        lsp_send(&session,json!({"jsonrpc":"2.0","id":shutdown_id,"method":"shutdown"}),deadline).await?;
        if !lsp_reply(&session,&mut capture,shutdown_id,deadline).await?.is_null(){return Err("Invalid LSP shutdown response".into());}
        stage="exit";
        lsp_send(&session,json!({"jsonrpc":"2.0","method":"exit"}),deadline).await?;
        drain(&session,&mut capture,deadline).await?;
        if !capture.exited||capture.code!=Some(0)||capture.signal.unwrap_or(0)!=0||!capture.closed{return Err("LSP shutdown failed".into());}
        Ok(projected)
    }.await;
    match result{
        Ok(mut d)=>{d["ok"]=json!(true);d["snapshotOnly"]=json!(true);d["transportClosed"]=json!(true);d},
        Err(error)=>{
            #[cfg(test)]
            eprintln!("CI LSP failure at {stage}: {error}; pending stderr: {}",String::from_utf8_lossy(&capture.stderr));
            #[cfg(not(test))]
            let _=(error,stage);
            let _=session.terminate().await;
            let closed=drain(&session,&mut capture,Instant::now()+TIMEOUT).await.is_ok()&&capture.closed;
            json!({"ok":false,"error":"LSP request rejected, unsupported, cancelled or timed out","transportClosed":closed})
        }
    }
}

// User-initiated local jobs in a separate disposable Ubuntu boot. No host
// workspace, native tool callbacks, provider credential or remote executor.
pub(crate) async fn local_process(guest: &IshInstance, env: &HashMap<String,String>, command: &str, workbench: bool) -> Value {
    use crate::local_jobs::{take_control, publish_output, Control};
    let mut opts = shell(command, env);
    opts.cwd = Some(Path::new("/tmp").to_path_buf());
    if !workbench {
        opts.tty = true;
        opts.pipe_stdin = false;
        opts.size = Some(PtySize{cols:80,rows:24});
    } else {
        opts.pipe_stdin = false;
    }
    let session = match guest.spawn(opts) {
        Ok(s) => s, Err(_) => return json!({"ok":false,"error":"Local process spawn failed"}),
    };
    let mut capture = Capture::default();
    capture.rolling_pty = !workbench;
    let deadline = Instant::now() + Duration::from_secs(1800);
    let mut stopped = false;
    let result: Result<(),String> = async {
        loop {
            for control in take_control() {
                match control {
                    Control::Stop => {
                        if !stopped && !capture.exited {
                            session.terminate().await.map_err(|_| "Stop failed".to_owned())?;
                            stopped = true;
                        }
                    }
                    Control::Input(bytes) if !workbench && !stopped && !capture.exited =>
                        write_ready(&session,&bytes,deadline).await?,
                    Control::Resize(cols,rows) if !workbench && !stopped && !capture.exited =>
                        session.resize(cols,rows).await.map_err(|_| "Resize failed".to_owned())?,
                    _ => {}
                }
            }
            capture.poll(&session, deadline).await?;
            // A stopped or failed process may have descendants. Only a clean
            // guest shutdown is final evidence of cleanup.
            if !workbench { publish_output(&capture.pty, !capture.exited && !capture.closed, capture.truncated_pty, capture.pty_total); }
            else {
                // Do not expose the server URL/token. Only a matching bind log
                // from the reviewed process permits the native loopback view.
                let bound = contains(&capture.stdout,b"Server bound to 127.0.0.1:18765") ||
                    contains(&capture.stderr,b"Server bound to 127.0.0.1:18765");
                publish_output(&[], bound && !capture.exited && !capture.closed, false, 0);
            }
            if capture.closed { break; }
        }
        Ok(())
    }.await;
    if result.is_err() {
        let _ = session.terminate().await;
        let _ = drain(&session,&mut capture,Instant::now()+TIMEOUT).await;
    }
    if !workbench { publish_output(&capture.pty,false,capture.truncated_pty,capture.pty_total); }
    #[cfg(test)]
    if workbench && std::env::var_os("NEXUS_WORKBENCH_CI_TRACE").is_some(){
        // The test token is synthetic; redact it anyway before logging.
        let output=capture.evidence().to_string();
        let output=env.get("NEXUS_WORKBENCH_TOKEN").filter(|s|!s.is_empty()).map_or_else(||output.clone(),|s|output.replace(s,"[redacted]"));
        eprintln!("CI Workbench server lifecycle: {output}");
    }
    json!({"ok":result.is_ok() && capture.closed && capture.exited,
        "stopped":stopped,"exitCode":capture.code,"termSignal":capture.signal,"outputTruncated":capture.truncated_pty,
        "closed":capture.closed,"error":if result.is_err() { Some("Local process deadline, output limit or transport failure") } else { None }})
}

pub(crate) async fn mcp_discover(guest: &IshInstance, env: &HashMap<String,String>, command: &str) -> Value {
    mcp_job(guest,env,command,None).await
}

pub(crate) async fn mcp_job(guest: &IshInstance, env: &HashMap<String,String>, command: &str, call: Option<(&str,&Value)>) -> Value {
    let mut opts = shell(command, env);
    opts.cwd = Some(Path::new("/tmp").to_path_buf());
    let session = match guest.spawn(opts) {
        Ok(s) => s, Err(_) => return json!({"ok":false,"error":"MCP process spawn failed"}),
    };
    let mut exchange = ACPExchange::new(session);
    let result: Result<Value,String> = async {
        let init = exchange.request(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"nexus-ios\",\"version\":\"0.4.1\"}}}\n",1).await?;
        if init.get("error").is_some() || init["result"]["protocolVersion"] != "2025-11-25" ||
            !init["result"]["capabilities"]["tools"].is_object() {
            return Err("MCP protocol/tools capability not supported".into());
        }
        exchange.notify(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await?;
        let list = exchange.request(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",2).await?;
        if list.get("error").is_some() || list["result"].get("nextCursor").is_some() {
            return Err("MCP error or unsupported pagination".into());
        }
        let tools = list["result"]["tools"].as_array().filter(|a| a.len() <= 32).ok_or("Invalid tool list")?;
        let mut names = Vec::new();
        for tool in tools {
            let name = tool["name"].as_str().filter(|s| !s.is_empty() && s.len() <= 128 &&
                s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))).ok_or("Invalid tool name")?;
            if !tool["inputSchema"].is_object() || names.iter().any(|n| n == name) {
                return Err("Invalid or duplicate MCP tool".into());
            }
            names.push(name.to_owned());
        }
        if let Some((name,args)) = call {
            if !names.iter().any(|n| n == name) { return Err("Tool not advertised".into()); }
            let request=json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":args}});
            let frame=exchange.request(format!("{request}\n").as_bytes(),3).await?;
            let r=&frame["result"];
            if frame.get("error").is_some() || !r.is_object() ||
                r.get("isError").is_some_and(|v| !v.is_boolean()) {
                return Err("Tool call rejected".into());
            }
            let blocks=r["content"].as_array().filter(|a| a.len()<=16).ok_or("Invalid content")?;
            let mut text=String::new();
            for block in blocks {
                if block["type"]!="text" { return Err("Unsupported tool content".into()); }
                let value=block["text"].as_str().ok_or("Invalid text content")?;
                if text.len()+value.len()+1>8192 || value.chars().any(|c| c.is_control() && c!='\n' && c!='\t') {
                    return Err("Tool output limit or control characters".into());
                }
                if !text.is_empty() {text.push('\n');}
                text.push_str(value);
            }
            let is_error=r["isError"]==true;
            Ok(json!({"ok":!is_error,"tools":names,"discoveryOnly":false,"calledTool":name,"toolError":is_error,"text":text}))
        } else { Ok(json!({"ok":true,"tools":names,"discoveryOnly":true})) }
    }.await;
    let closed = exchange.close().await;
    match result {
        Ok(mut report) => {
            report["ok"]=json!(report["ok"]==true && closed);
            report["transportClosed"]=json!(closed);
            report["protocolVersion"]=json!("2025-11-25");
            report
        },
        Err(_) => json!({"ok":false,"transportClosed":closed,"error":"MCP request rejected or incomplete; tool may already have executed"}),
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
