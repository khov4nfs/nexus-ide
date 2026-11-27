#ifndef NEXUS_GUEST_BRIDGE_H
#define NEXUS_GUEST_BRIDGE_H
#ifdef __cplusplus
extern "C" {
#endif
// Blocking, single attempt per process. Caller releases returned JSON via nexus_guest_free.
char *nexus_guest_probe(const char *rootfs_data_path);
// Explicit network opt-in: fixed signed Alpine jq package installation probe.
char *nexus_guest_probe_packages(const char *rootfs_data_path);
// Explicit consent for larger fixed development toolchain downloads.
char *nexus_guest_probe_toolchains(const char *rootfs_data_path);
// Offline text-only transfer, max 2048 UTF-8 bytes, no embedded NUL.
char *nexus_guest_probe_workspace(const char *rootfs_data_path, const char *text);
// Ubuntu compatibility; NULL digest means no AGY execution.
char *nexus_guest_probe_glibc(const char *rootfs_data_path, const char *expected_agy_sha256);
// Native initialize request only; requires non-NULL digest and request.
char *nexus_guest_probe_glibc_acp(const char *rootfs_data_path, const char *expected_agy_sha256, const char *initialize_request);
// Blocking calls: serialize off the main thread. Idle timeout: 180 seconds.
char *nexus_acp_start(const char *rootfs_data_path, const char *expected_agy_sha256, const char *initialize_request);
// Explicit consent required; key is passed only to the agent process environment.
char *nexus_acp_start_gemini(const char *rootfs_data_path, const char *expected_agy_sha256, const char *initialize_request, const char *key);
// Google terminal auth: one worker owns the login and subsequent ACP process.
char *nexus_acp_start_google(const char *rootfs_data_path, const char *expected_agy_sha256, const char *initialize_request);
char *nexus_google_login_progress(void);
char *nexus_google_login_code(const char *code);
char *nexus_google_login_cancel(void);
char *nexus_acp_connection_status(void);
char *nexus_acp_new_session(const char *session_new_request);
// Explicit model-network consent; one fixed prompt, no arbitrary content.
char *nexus_acp_prompt_probe(void);
// Up to three consented text turns (max 1024 UTF-8 bytes each); no project mount.
// Smoke test and chat are mutually exclusive within one boot.
char *nexus_acp_chat(const char *text);
// Concurrent-safe status and cancel request; does not wait for completion.
char *nexus_acp_prompt_progress(void);
char *nexus_acp_cancel_prompt(void);
char *nexus_acp_close(void);
// Separate local boot: 1 PTY, 2 MCP discovery, 3 token-protected workbench.
#include <stdint.h>
char *nexus_local_run(const char *rootfs_data_path, uint8_t mode, const char *input);
char *nexus_local_progress(void);
char *nexus_local_input(const char *input);
char *nexus_local_resize(uint16_t cols, uint16_t rows);
char *nexus_local_stop(void);
void nexus_guest_free(char *result);
#ifdef __cplusplus
}
#endif
#endif
