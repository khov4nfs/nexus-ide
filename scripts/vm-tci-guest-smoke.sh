#!/bin/sh
# Runs ONLY as PID 1 in the disposable full-system Linux VM.
# No network, registry pulls, user data or host Docker socket.
set -eu
test "$$" -eq 1
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
export HOME=/root
export DOCKER_HOST=unix:///run/docker.sock
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH
daemon=
finish() {
    rc=$?
    trap - EXIT
    if [ -n "$daemon" ]; then
        kill -TERM "$daemon" 2>/dev/null || true
        wait "$daemon" 2>/dev/null || true
    fi
    if [ "$rc" -ne 0 ]; then
        echo "NEXUS_VM_FAIL:$rc"
        tail -n 60 /run/nexus-dockerd.log 2>/dev/null || true
    fi
    sync
    busybox poweroff -f
    # Never return from PID 1 if poweroff failed.
    while :; do sleep 10; done
}
trap finish EXIT
mountpoint -q /proc || mount -t proc proc /proc
mountpoint -q /sys || mount -t sysfs sysfs /sys
mountpoint -q /dev || mount -t devtmpfs devtmpfs /dev
mkdir -p /run /dev/pts /dev/shm /sys/fs/cgroup /tmp
mountpoint -q /run || mount -t tmpfs tmpfs /run
mountpoint -q /dev/pts || mount -t devpts devpts /dev/pts
mountpoint -q /dev/shm || mount -t tmpfs tmpfs /dev/shm
mountpoint -q /sys/fs/cgroup || mount -t cgroup2 cgroup2 /sys/fs/cgroup
test -r /sys/fs/cgroup/cgroup.controllers
test "$(uname -m)" = aarch64
test -r /proc/self/ns/mnt
test -r /proc/self/ns/pid
test -r /proc/self/ns/net
test "$(cat /proc/1/comm)" != ""
echo "NEXUS_VM_KERNEL:$(uname -r)"
echo "NEXUS_VM_CGROUP2_OK"
dpkg-query -W docker.io docker-compose-v2 busybox-static
# Deliberately use the vfs driver and no networking; these are a
# prerequisite smoke test, not evidence for overlay2 or port forwarding.
dockerd --host "$DOCKER_HOST" --data-root /var/lib/docker \
    --exec-root /run/docker --pidfile /run/docker.pid \
    --exec-opt native.cgroupdriver=cgroupfs --storage-driver vfs \
    --iptables=false --ip6tables=false --bridge=none --ip-forward=false \
    --ip-masq=false --userland-proxy=false >/run/nexus-dockerd.log 2>&1 &
daemon=$!
i=0
until docker info >/dev/null 2>&1; do
    kill -0 "$daemon"
    i=$((i+1))
    test "$i" -lt 240
    sleep 1
done
test "$(docker info --format '{{.Driver}}')" = vfs
test "$(docker info --format '{{.CgroupVersion}}')" = 2
echo NEXUS_VM_DOCKER_DAEMON_OK
# The image is made from a signed Ubuntu busybox-static package, not a
# registry pull. The container has its own namespaces and no network.
mkdir -p /tmp/nexus-image/bin
cp "$(command -v busybox)" /tmp/nexus-image/bin/busybox
tar --numeric-owner -C /tmp/nexus-image -cf - . | docker import - nexus-vm-smoke:local >/dev/null
test "$(docker run --pull=never --network none --rm nexus-vm-smoke:local /bin/busybox echo NEXUS_CONTAINER_42)" = NEXUS_CONTAINER_42
echo NEXUS_VM_DOCKER_RUN_OK
mkdir -p /tmp/nexus-compose
cd /tmp/nexus-compose
cat >compose.yaml <<'YAML'
services:
  producer:
    image: nexus-vm-smoke:local
    network_mode: none
    command: ["/bin/busybox", "sh", "-c", "echo NEXUS_COMPOSE_42 > /shared/proof"]
    volumes: ["proof:/shared"]
  consumer:
    image: nexus-vm-smoke:local
    network_mode: none
    command: ["/bin/busybox", "cat", "/shared/proof"]
    depends_on:
      producer:
        condition: service_completed_successfully
    volumes: ["proof:/shared:ro"]
volumes:
  proof:
YAML
docker compose version
docker compose -p nexus-vm up -d --no-build --pull never
producer=$(docker compose -p nexus-vm ps -a -q producer)
consumer=$(docker compose -p nexus-vm ps -a -q consumer)
test -n "$producer" && test -n "$consumer" && test "$producer" != "$consumer"
test "$(docker wait "$producer")" = 0
test "$(docker wait "$consumer")" = 0
test "$(docker logs "$consumer")" = NEXUS_COMPOSE_42
docker compose -p nexus-vm down --volumes
test -z "$(docker ps -aq)"
echo NEXUS_VM_COMPOSE_OK
kill -TERM "$daemon"
wait "$daemon"
daemon=
sync
echo NEXUS_VM_CLEAN_SHUTDOWN
