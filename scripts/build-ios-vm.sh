#!/usr/bin/env bash
# Compile a device-only, in-process QEMU TCI candidate. No IPA publication.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD"
test "$(uname -s)" = Darwin && test "$(uname -m)" = arm64
utm_pin=3d4b8d8cb1ce93a8b1a9610695c12487685f0c15
qemu_pin=6601422e1fff2da1376faafb1e4c2c5cdb2d8003
mkdir -p Vendor RuntimeAssets
test ! -e Vendor/nexus-utm && test ! -e Vendor/nexus-qemu && test ! -e RuntimeAssets/full-vm
git clone --no-checkout https://github.com/utmapp/UTM.git Vendor/nexus-utm
git -C Vendor/nexus-utm checkout --detach "$utm_pin"
git clone --no-checkout https://github.com/utmapp/qemu.git Vendor/nexus-qemu
git -C Vendor/nexus-qemu checkout --detach "$qemu_pin"
test "$(git -C Vendor/nexus-qemu rev-parse HEAD)" = "$qemu_pin"
git -C Vendor/nexus-qemu submodule update --init --recursive
# Apply the exact patch used by the pinned UTM dependency build.
patch_file="$root/Vendor/nexus-utm/patches/qemu-10.0.12-utm.patch"
git -C Vendor/nexus-qemu apply --check "$patch_file"
git -C Vendor/nexus-qemu apply "$patch_file"
# Use only the reviewed iOS cross-build infrastructure, not UTM UI or
# private hypervisor support. Every substitution requires exact context.
python3 - "$root" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
p = root / "Vendor/nexus-utm/scripts/build_dependencies.sh"
s = p.read_text()
def replace(a, b):
    global s
    if s.count(a) != 1:
        raise SystemExit("Unexpected pinned UTM build context: " + a[:80])
    s = s.replace(a, b)
def function(name, body):
    global s
    start = s.index(name + " () {")
    end = s.index("\n}\n", start) + 2
    s = s[:start] + name + " () {\n" + body + "\n}" + s[end:]
function("download_all", """    mkdir -p "$BUILD_DIR"
    download $PKG_CONFIG_SRC
    download $FFI_SRC
    download $ICONV_SRC
    download $GETTEXT_SRC
    download $GLIB_SRC
    download $PIXMAN_SRC
    clone $LIBUCONTEXT_REPO $LIBUCONTEXT_COMMIT""")
function("build_qemu_dependencies", """    build $FFI_SRC
    build $ICONV_SRC
    gl_cv_onwards_func_strchrnul=future build $GETTEXT_SRC --disable-java
    meson_build $GLIB_SRC -Dtests=false -Ddtrace=disabled
    build $PIXMAN_SRC
    meson_build $LIBUCONTEXT_REPO -Ddefault_library=static -Dfreestanding=true""")
replace('    QEMU_PLATFORM_BUILD_FLAGS="--enable-shared-lib --disable-cocoa --disable-sdl --disable-coreaudio --disable-slirp-smbd --enable-ucontext --with-coroutine=libucontext $HVF_FLAGS $TCI_BUILD_FLAGS"',
        '    QEMU_PLATFORM_BUILD_FLAGS="--without-default-features --enable-tcg --enable-tcg-interpreter --target-list=aarch64-softmmu --enable-fdt=internal --enable-shared-lib --disable-hvf --disable-hvf-private --disable-kvm --disable-plugins --disable-docs --disable-cocoa --disable-sdl --enable-ucontext --with-coroutine=libucontext"')
replace('\ncopy_private_headers\nbuild_pkg_config\n', '\nbuild_pkg_config\n')
replace('\nbuild_spice_client\nbuild_vulkan_drivers\nbuild_d3d_drivers\nfixup_all\nremove_shared_gst_plugins # another hack...\n','\nfixup_all\n')
# Override only the platform SDK floor; do not use private headers/entitlements.
replace('IOS_SDKMINVER="15.0"', 'IOS_SDKMINVER="17.0"')
p.write_text(s)
q = root / "Vendor/nexus-qemu/system/vl.c"
source = q.read_text()
assert source.count("void qemu_init(int argc, char **argv)") == 1
source += r'''
/* Nexus device-only integration entrypoint; keep QEMU resident until app exit.
 * qemu_init takes the BQL and replay lock, as in system/main.c.
 * No exit(), dlclose(), or native JIT in the normal lifecycle.
 * Fatal upstream errors may still terminate the app; disk remains dirty.
 */
#if defined(CONFIG_SHARED_LIBRARY_BUILD)
#if !defined(CONFIG_TCG_INTERPRETER)
#error Nexus requires portable TCI, not native TCG
#endif
#if defined(CONFIG_HVF_PRIVATE)
#error Nexus forbids private Hypervisor
#endif
__attribute__((visibility("default")))
int nexus_qemu_tci_run(int argc, char **argv)
{
    qemu_init(argc, argv);
    int status = qemu_main_loop();
    qemu_cleanup(status);
    bql_unlock();
    replay_mutex_unlock();
    return status;
}
#endif
'''
q.write_text(source)
PY
# UTM's script builds its pinned dependency versions; source/dependency
# redistribution and exact archive integrity review are still release gates.
(
  cd Vendor/nexus-utm
  NCPU=2 bash scripts/build_dependencies.sh -p ios-tci -a arm64 -q "$root/Vendor/nexus-qemu"
)
sysroot="$root/Vendor/nexus-utm/sysroot-iOS-TCI-arm64"
test -d "$sysroot/Frameworks"
python3 scripts/stage-ios-vm.py "$sysroot" "$root/RuntimeAssets/full-vm" "$utm_pin" "$qemu_pin"
