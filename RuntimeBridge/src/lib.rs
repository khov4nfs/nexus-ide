//! Experimental in-process guest smoke probe, not an isolation boundary.
use ish_embed_host::IshInstance;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static ATTEMPTED: AtomicBool = AtomicBool::new(false);

fn probe(path: &Path) -> Value {
    if !path.is_dir() || !path.parent().map(|p| p.join("meta.db").is_file()).unwrap_or(false) {
        return json!({"ok":false,"stage":"rootfs","error":"Expected fakefs data directory with sibling meta.db"});
    }
    // Upstream kernel globals make rebooting in the same process unsafe.
    if ATTEMPTED.swap(true, Ordering::AcqRel) {
        return json!({"ok":false,"stage":"boot","error":"Restart the app before another guest boot"});
    }
    let guest = match IshInstance::boot(path, Some(Path::new("/"))) {
        Ok(g) => g,
        Err(e) => return json!({"ok":false,"stage":"boot","error":e.to_string()}),
    };
    let env = HashMap::from([
        ("PATH".to_owned(), "/usr/bin:/bin:/usr/sbin:/sbin".to_owned()),
        ("HOME".to_owned(), "/root".to_owned()),
        ("LANG".to_owned(), "C".to_owned()),
    ]);
    // Intentionally fixed, trusted commands; no arbitrary project scripts.
    let commands: [(&str, &[&str], i32, &str); 3] = [
        ("architecture", &["/bin/uname", "-m"], 0, "aarch64"),
        ("stdout", &["/bin/echo", "NEXUS_GUEST_OK"], 0, "NEXUS_GUEST_OK"),
        ("shell_exit", &["/bin/sh", "-c", "printf 'NEXUS_SHELL_OK\\n'; exit 7"], 7, "NEXUS_SHELL_OK"),
    ];
    let mut rows = Vec::new();
    let mut ok = true;
    for (name, args, expected_code, expected_text) in commands {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (code, bytes) = guest.run_oneshot(&argv, Path::new("/"), &env, Some(10_000));
        let text = String::from_utf8_lossy(&bytes);
        let passed = code == expected_code && text.trim() == expected_text;
        ok &= passed;
        rows.push(json!({"name":name,"exitCode":code,"output":text.chars().take(4096).collect::<String>(),"passed":passed}));
    }
    let shutdown = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt.block_on(guest.shutdown()).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    ok &= shutdown.is_ok();
    json!({"ok":ok,"stage":"probe","checks":rows,"shutdownError":shutdown.err()})
}

/// # Safety
/// path must point to a valid, null-terminated string for this call.
#[no_mangle]
pub unsafe extern "C" fn nexus_guest_probe(path: *const c_char) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if path.is_null() {
            return json!({"ok":false,"stage":"rootfs","error":"Missing rootfs"});
        }
        match CStr::from_ptr(path).to_str() {
            Ok(s) => probe(Path::new(s)),
            Err(_) => json!({"ok":false,"stage":"rootfs","error":"Invalid UTF-8 path"}),
        }
    }).unwrap_or_else(|_| json!({"ok":false,"stage":"bridge","error":"Rust panic; restart application"}));
    // Serialized JSON escapes embedded NUL bytes.
    CString::new(result.to_string()).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// result must be null or a pointer returned by nexus_guest_probe, freed once.
#[no_mangle]
pub unsafe extern "C" fn nexus_guest_free(result: *mut c_char) {
    if !result.is_null() { drop(CString::from_raw(result)); }
}
