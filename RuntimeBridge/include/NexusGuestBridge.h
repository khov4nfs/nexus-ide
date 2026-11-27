#ifndef NEXUS_GUEST_BRIDGE_H
#define NEXUS_GUEST_BRIDGE_H
#ifdef __cplusplus
extern "C" {
#endif
// Blocking, single attempt per process. Caller owns returned JSON via free.
char *nexus_guest_probe(const char *rootfs_data_path);
void nexus_guest_free(char *result);
#ifdef __cplusplus
}
#endif
#endif
