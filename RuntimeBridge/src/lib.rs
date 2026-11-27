//! Experimental in-process guest smoke probe, not an isolation boundary.
use ish_embed_host::IshInstance;
mod process_probe;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static ATTEMPTED: AtomicBool = AtomicBool::new(false);

fn probe(path: &Path, packages: bool, toolchains: bool, workspace_text: &str) -> Value {
    if !process_probe::workspace_input_ok(workspace_text) { return json!({"ok":false,"stage":"workspace","error":"Expected UTF-8 text of at most 2048 bytes without NUL"}); }
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
        Ok(rt) => rt.block_on(async {
            let process_rows = process_probe::run(&guest, &env).await;
            ok &= process_rows.iter().all(|row| row["passed"].as_bool() == Some(true));
            rows.extend(process_rows);
            if packages && ok {
                let row = process_probe::package_install(&guest, &env).await;
                ok &= row["passed"].as_bool() == Some(true);
                rows.push(row);
            }
            if toolchains && ok {
                let row = process_probe::toolchain_install(&guest, &env).await;
                ok &= row["passed"].as_bool() == Some(true);
                rows.push(row);
            }
            if ok {
                let row = process_probe::workspace_roundtrip(&guest, &env, workspace_text).await;
                ok &= row["passed"].as_bool() == Some(true);
                rows.push(row);
            }
            guest.shutdown().await.map_err(|e| e.to_string())
        }),
        Err(e) => Err(e.to_string()),
    };
    ok &= shutdown.is_ok();
    json!({"ok":ok,"stage":"probe","packageRequested":packages,"toolchainsRequested":toolchains,"checks":rows,"shutdownError":shutdown.err()})
}

/// # Safety
/// path must point to a valid, null-terminated string for this call.
#[no_mangle]
pub unsafe extern "C" fn nexus_guest_probe(path: *const c_char) -> *mut c_char {
    run_bridge(path, false, false, process_probe::WORKSPACE_FIXTURE)
}

/// # Safety
/// Same pointer contract as nexus_guest_probe. Only call after explicit network consent.
#[no_mangle]
pub unsafe extern "C" fn nexus_guest_probe_packages(path: *const c_char) -> *mut c_char {
    run_bridge(path, true, false, process_probe::WORKSPACE_FIXTURE)
}

/// # Safety
/// Same pointer contract as nexus_guest_probe; explicit large download consent.
#[no_mangle]
pub unsafe extern "C" fn nexus_guest_probe_toolchains(path: *const c_char) -> *mut c_char {
    run_bridge(path, true, true, process_probe::WORKSPACE_FIXTURE)
}

/// # Safety
/// Both pointers must reference valid NUL-terminated strings for this call.
/// Caller must explicitly approve transferring the selected text to the local guest.
#[no_mangle]
pub unsafe extern "C" fn nexus_guest_probe_workspace(path: *const c_char, text: *const c_char) -> *mut c_char {
    if text.is_null() {
        return CString::new(r#"{"ok":false,"stage":"workspace","error":"Missing text"}"#).unwrap().into_raw();
    }
    match CStr::from_ptr(text).to_str() {
        Ok(text) => run_bridge(path, false, false, text),
        Err(_) => CString::new(r#"{"ok":false,"stage":"workspace","error":"Invalid UTF-8"}"#).unwrap().into_raw(),
    }
}

unsafe fn run_bridge(path: *const c_char, packages: bool, toolchains: bool, workspace_text: &str) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if path.is_null() {
            return json!({"ok":false,"stage":"rootfs","error":"Missing rootfs"});
        }
        match CStr::from_ptr(path).to_str() {
            Ok(s) => probe(Path::new(s), packages, toolchains, workspace_text),
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

#[cfg(test)]
mod tests {
    use super::*;

    // Explicit opt-in host execution gate, never a silent pass without rootfs.
    #[test]
    #[ignore = "requires freshly generated trusted ARM64 fakefs"]
    fn real_guest_process_probe() {
        let path = std::env::var_os("NEXUS_TEST_ROOTFS").expect("set NEXUS_TEST_ROOTFS to fakefs/data");
        let evidence_path = std::env::var_os("NEXUS_TEST_EVIDENCE").expect("set NEXUS_TEST_EVIDENCE");
        let packages = std::env::var("NEXUS_TEST_PACKAGES").as_deref() == Ok("1");
        let toolchains = std::env::var("NEXUS_TEST_TOOLCHAINS").as_deref() == Ok("1");
        assert!(!toolchains || packages, "toolchain mode requires packages");
        let report = probe(Path::new(&path), packages, toolchains, process_probe::WORKSPACE_FIXTURE);
        std::fs::write(evidence_path, serde_json::to_vec_pretty(&report).unwrap()).expect("write completed evidence");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert_eq!(report["ok"].as_bool(), Some(true), "{report}");
        assert_eq!(report["checks"].as_array().map(Vec::len), Some(if toolchains { 9 } else if packages { 8 } else { 7 }));
    }
}
