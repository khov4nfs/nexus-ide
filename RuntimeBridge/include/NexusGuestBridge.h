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
void nexus_guest_free(char *result);
#ifdef __cplusplus
}
#endif
#endif
