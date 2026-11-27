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
/sbin/apk info -v jq
/usr/bin/jq --version
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
    matches!((absent, installed), (Some(a), Some(b)) if a < b) &&
        lines.iter().any(|line| line.starts_with("jq-") && line.contains("-r")) &&
        lines.iter().any(|line| line.starts_with("jq-") && !line.contains("-r")) &&
        lines.last() == Some(&"NEXUS_JQ_OK")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_evidence_requires_install_and_execution() {
        let good = b"NEXUS_PACKAGE_ABSENT\nNEXUS_PACKAGE_INSTALLED\njq-1.7.1-r0\njq-1.7.1\nNEXUS_JQ_OK\n";
        assert!(package_output_ok(good));
        assert!(!package_output_ok(b"NEXUS_JQ_OK\n"));
        assert!(!package_output_ok(b"NEXUS_PACKAGE_INSTALLED\nNEXUS_PACKAGE_ABSENT\njq-1.7.1-r0\njq-1.7.1\nNEXUS_JQ_OK\n"));
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
