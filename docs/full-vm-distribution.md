# Full-system VM distribution review (blocking, not approved)

## Actual source review

Repository rules (AGENTS.md) require upstream notices and evidence. Nexus root LICENSE was absent at the time of this review; no license is selected by this document. Do not distribute a QEMU-linked IPA until the owner has reviewed the combined-work licensing, including the already integrated runtime. Dynamic loading in the same process does not by itself establish a GPL exception.

| Component | Evidence / initial findings | Required action |
|---|---|---|
| QEMU 10.0.12 UTM fork | [LICENSE](https://github.com/utmapp/qemu/blob/6601422e1fff2da1376faafb1e4c2c5cdb2d8003/LICENSE), [COPYING](https://github.com/utmapp/qemu/blob/6601422e1fff2da1376faafb1e4c2c5cdb2d8003/COPYING): emulator as a whole GPLv2; TCG code has mixed file-specific licenses. | Review combined work, distribute complete corresponding source with modifications/build scripts and required license notices. |
| UTM build tooling | [Apache-2.0 LICENSE](https://github.com/utmapp/UTM/blob/3d4b8d8cb1ce93a8b1a9610695c12487685f0c15/LICENSE); scripts are patched for the Nexus build. | Preserve notices and review applicable compatibility of any code actually linked (not merely host build scripts). |
| GLib 2.83.0 | [COPYING LGPL 2.1](https://github.com/GNOME/glib/blob/2.83.0/COPYING) | Preserve license, corresponding library source/modifications and relinking rights where applicable. |
| libffi 3.5.0 | [MIT-style LICENSE](https://github.com/libffi/libffi/blob/v3.5.0/LICENSE) | Preserve copyright and permission text. |
| libucontext | [ISC-style LICENSE](https://github.com/utmapp/libucontext/blob/9b1d8f01a6e99166f9808c79966abe10786de8b6/LICENSE) | Preserve notice; include exact fork source as required. |
| libslirp 4.9.1 | [COPYRIGHT](https://github.com/utmapp/libslirp/blob/v4.9.1-release-mirror/COPYRIGHT): BSD-style terms with other file-specific copyrights. | Include actual copyright notices and review each bundled component. |
| libiconv 1.16, gettext 0.22.5, pixman 0.38.0, pkg-config 0.29.2 | Pinned inputs in [UTM sources](https://github.com/utmapp/UTM/blob/3d4b8d8cb1ce93a8b1a9610695c12487685f0c15/patches/sources). | Inspect downloaded archive notices, distinguish host tools from linked libraries, review any static objects and relinking/source obligations. Not yet fully reviewed. |
| QEMU firmware / Meson subprojects | [pc-bios README](https://github.com/utmapp/qemu/blob/6601422e1fff2da1376faafb1e4c2c5cdb2d8003/pc-bios/README) lists separately licensed firmware. | Review every actually shipped firmware hash and subproject source; don't assume QEMU COPYING covers firmware. |
| Existing user-mode runtime / Rust | Pinned litter-ish and RuntimeBridge Cargo.toml. Exact Cargo.lock was not present in repository during review. | Inventory resolved Rust packages, C runtime dependencies, upstream notices, patches and complete corresponding source. Root LICENSE lookup for the pinned litter-ish revision was inconclusive. |
| SwiftTerm 1.19.0 | [MIT-style LICENSE](https://github.com/migueldeicaza/SwiftTerm/blob/464df5207fc2432e16c9a23abe538187196daf5f/LICENSE) | Preserve existing bundled license and provenance. |
| Ubuntu rootfs / kernel / Docker / Compose / BusyBox | Actual signed Ubuntu package versions are exported in packages.tsv for the full VM. | Review per-package copyright/source (including installed dependencies). VM disk is NOT bundled in IPA or published by CI. |
| OpenVSCode / Node and dependencies | Pinned source commit and upstream LICENSE/ThirdPartyNotices already staged for workbench. | Review exact binary release and bundled components, source obligations. |
| AGY, Codex, Claude | User imports reviewed permitted executables; not redistributed by this workflow. | Keep this policy; do not include proprietary binaries or credentials. |

The new iOS CI produces source-inventory.json, provenance.json and DependencyNotices. These are review inputs, not a completed SBOM/legal opinion: static transitive components, firmware-specific rights, the prior runtime and guest package notices need further human verification. No assertion that all licenses are compatible has been made. The repo owner must decide the applicable distribution license and source availability. If constraints cannot be reconciled, keep the full VM build for private research and do not publish the combined app. This is a technical compliance checklist, not legal advice.

## Final workflow

The ordinary workflow default remains unchanged. The optional full_vm flag is wired into Experimental iOS guest build on main only. Selecting it also requires license_reviewed=true, an owner-authored docs/full-vm-distribution-review.json and a root LICENSE. The record must have:
- ownerApproved, combinedWorkLicenseReviewed, allDependenciesReviewed, correspondingSourceReviewed: explicit true only after actual review;
- repositoryLicenseSHA256, sourceInventorySHA256, sourceArchiveSHA256: exact lowercase SHA-256 values;
- sourceArchiveURL: stable public HTTPS URL for the complete corresponding-source archive.

No approval record is created automatically. The same workflow requires successful host TCI Docker/Compose and network regression, builds the actual iPhoneOS framework, verifies the exact inventory and downloaded corresponding-source archive, sets NEXUS_FULL_VM for Release, then checks the IPA framework and notices. It binds additional VM evidence and adds physical iPhone VM checks to the untouched device template. A build success is not device acceptance.

## Prepare a local import image

On a trusted ARM64 Linux build host with Docker, sudo, e2fsprogs, Python and curl:

    bash scripts/prepare-full-vm-image.sh ./nexus-vm-import
    cd nexus-vm-import
    sha256sum -c SHA256SUMS

The script prepares Ubuntu ARM64 kernel (raw Image), initrd, 4 GiB raw ext4 disk, package inventory and SHA256SUMS. Packages are retrieved from signed Ubuntu repositories; exact selected versions are recorded, not claimed bit-for-bit reproducible. It includes the test PID 1 and Docker/Compose. The host regression uses a copy of the disk, not the original import artifact. CI does not publish the image. Review guest package source/license obligations before transferring/distributing it.

Import kernel, initrd and disk separately in the Full VM tab with corresponding SHA-256. For this prepared image, enable the acceptance-boot option and choose 1024 MiB. Enable restricted network only for the network test. After NEXUS_VM_FORWARD_READY, tap Verify local container HTTP. The expected guest markers include actual Docker run, Compose volume, Compose internal DNS/HTTP, host loopback forwarding and clean shutdown. The network mode deliberately has no Internet or LAN egress and cannot pull registry images; this is not general network acceptance. Do not use it to expose sensitive services. Ordinary guest boot requires a separately configured guest console/login; this test image's primary intended use is explicit acceptance boot.

Physical iPhone acceptance must separately test re-launch and persistent disk, clean poweroff, resource pressure, signed framework loading and real providers. No final IPA dispatch is performed by this change.
