#!/bin/sh
# Runs ONLY as PID 1 in the disposable full-system Linux VM.
# Guest-only Docker/containerd. Restricted SLIRP on nexus.net=1; registry pull only on nexus.net=2.
set -eu
test "$$" -eq 1
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
export HOME=/root
export DOCKER_HOST=unix:///run/docker.sock
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH
daemon=
containerd_pid=
phase=boot
finish() {
    rc=$?
    trap - EXIT
    if [ "$rc" -ne 0 ]; then
        echo "NEXUS_VM_FAIL:$rc"
        echo "NEXUS_VM_FAIL_PHASE:$phase"
    fi
    if [ -n "$daemon" ]; then
        kill -TERM "$daemon" 2>/dev/null || true
        wait "$daemon" 2>/dev/null || true
    fi
    if [ -n "$containerd_pid" ]; then
        kill -TERM "$containerd_pid" 2>/dev/null || true
        wait "$containerd_pid" 2>/dev/null || true
    fi
    if [ "$rc" -ne 0 ]; then
        echo "Last Docker readiness probe:"
        tail -n 20 /run/nexus-docker-probe.log 2>/dev/null || true
        echo "Last Docker info:"
        tail -n 20 /run/nexus-docker-info.log 2>/dev/null || true
        echo "Guest containerd log:"
        tail -n 100 /run/nexus-containerd.log 2>/dev/null || true
        echo "Guest dockerd log:"
        tail -n 100 /run/nexus-dockerd.log 2>/dev/null || true
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
mountpoint -q /tmp || mount -t tmpfs -o mode=1777,size=64m tmpfs /tmp
mountpoint -q /dev/pts || mount -t devpts devpts /dev/pts
mountpoint -q /dev/shm || mount -t tmpfs tmpfs /dev/shm
mountpoint -q /sys/fs/cgroup || mount -t cgroup2 cgroup2 /sys/fs/cgroup
test -r /sys/fs/cgroup/cgroup.controllers
test "$(uname -m)" = aarch64
test -r /proc/self/ns/mnt
test -r /proc/self/ns/pid
test -r /proc/self/ns/net
test "$(cat /proc/1/comm)" != ""
mkdir -p /var/lib/nexus-vm-acceptance
counter=/var/lib/nexus-vm-acceptance/boot-count
previous=0
if [ -e "$counter" ]; then previous=$(cat "$counter"); fi
case "$previous" in ""|*[!0-9]*) exit 1;; esac
test "$previous" -lt 9999
echo "NEXUS_VM_PERSIST_PREVIOUS:$previous"
echo "NEXUS_VM_KERNEL:$(uname -r)"
echo "NEXUS_VM_CGROUP2_OK"
dpkg-query -W docker.io docker-compose-v2 containerd busybox-static
# Retain the vfs prerequisite. In network mode verify actual bridge NAT
# and published ports, not a host Docker daemon or fake proxy.
case " $(cat /proc/cmdline) " in
    *" nexus.net=1 "*|*" nexus.net=2 "*)
        network=1
        case " $(cat /proc/cmdline) " in *" nexus.net=2 "*) network=2;; esac
        modprobe virtio_net || true
        i=0
        iface=
        while [ -z "$iface" ]; do
            for candidate in /sys/class/net/*; do
                [ "${candidate##*/}" = lo ] || iface="${candidate##*/}"
            done
            i=$((i+1)); test "$i" -lt 30
            [ -n "$iface" ] || sleep 1
        done
        ip link set lo up
        ip link set "$iface" up
        ip addr add 10.0.2.15/24 dev "$iface"
        ip route replace default via 10.0.2.2
        printf "nameserver 10.0.2.3\n" >/etc/resolv.conf
        if [ "$network" -eq 1 ]; then
            if curl --proto "=https" --silent --show-error --max-time 15 https://registry-1.docker.io/v2/ -o /dev/null; then
                echo "Restricted mode unexpectedly reached registry" >&2; exit 1
            fi
            echo NEXUS_VM_RESTRICTED_EGRESS_BLOCKED
        fi
        if [ "$network" -eq 2 ]; then
            curl --proto "=https" --tlsv1.2 --fail --silent --show-error --max-time 90 https://registry-1.docker.io/v2/ -o /tmp/registry-response -w "%{http_code}" > /tmp/registry-status || test "$(cat /tmp/registry-status)" = 401
            test "$(cat /tmp/registry-status)" = 401
            echo NEXUS_VM_INTERNET_TLS_OK
        fi
        set -- --iptables=true --ip6tables=false --ip-forward=true --ip-masq=true --userland-proxy=false
        echo NEXUS_VM_NET_CONFIG_OK
        ;;
    *)
        network=0
        set -- --iptables=false --ip6tables=false --bridge=none --ip-forward=false --ip-masq=false --userland-proxy=false
        ;;
esac
# Under interpreter-only TCI, the default dockerd-managed containerd
# bootstrap timeout is too short for a cold guest. Start the real guest
# service independently and wait for a successful gRPC Version response.
# Keep both daemons on guest-local UNIX sockets, with separate state roots.
phase=containerd_start
containerd_socket=/run/nexus-containerd.sock
printf 'version = 3\n' >/run/nexus-containerd.toml
containerd --config /run/nexus-containerd.toml \
    --address "$containerd_socket" --root /var/lib/containerd --state /run/nexus-containerd \
    >/run/nexus-containerd.log 2>&1 &
containerd_pid=$!
echo NEXUS_VM_CONTAINERD_START
i=0
until [ -S "$containerd_socket" ] && timeout 30s ctr --address "$containerd_socket" version >/run/nexus-containerd-version.txt 2>&1 &&
    grep -q '^Server:' /run/nexus-containerd-version.txt; do
    kill -0 "$containerd_pid"
    i=$((i+1))
    test "$i" -lt 20
    sleep 3
done
echo NEXUS_VM_CONTAINERD_READY
# Explicitly preserve the existing classic vfs driver contract for Docker 29.
printf '{"features":{"containerd-snapshotter":false}}\n' >/run/nexus-dockerd.json
phase=dockerd_start
dockerd --config-file /run/nexus-dockerd.json --containerd "$containerd_socket" \
    --host "$DOCKER_HOST" --data-root /var/lib/docker \
    --exec-root /run/docker --pidfile /run/docker.pid \
    --exec-opt native.cgroupdriver=cgroupfs --storage-driver vfs \
    "$@" >/run/nexus-dockerd.log 2>&1 &
daemon=$!
# A lightweight HTTP ping avoids repeatedly starting the Go CLI and
# collecting expensive system info on a single interpreter CPU.
phase=dockerd_ping
i=0
until [ -S /run/docker.sock ] &&
    curl --unix-socket /run/docker.sock --noproxy '*' --fail --silent --show-error \
        --connect-timeout 5 --max-time 30 http://localhost/_ping \
        >/run/nexus-docker-ping.txt 2>/run/nexus-docker-probe.log &&
    [ "$(cat /run/nexus-docker-ping.txt)" = OK ]; do
    kill -0 "$daemon"
    kill -0 "$containerd_pid"
    i=$((i+1))
    test "$i" -lt 120
    sleep 5
done
echo NEXUS_VM_DOCKER_API_READY
# Still query the actual daemon for both properties, once, with a
# realistic TCI deadline. API ping alone is not sufficient evidence.
phase=dockerd_info
timeout 180s docker info --format '{{.Driver}} {{.CgroupVersion}}' \
    >/run/nexus-docker-info.txt 2>/run/nexus-docker-info.log
test "$(cat /run/nexus-docker-info.txt)" = 'vfs 2'
kill -0 "$daemon"
kill -0 "$containerd_pid"
echo NEXUS_VM_DOCKER_DAEMON_OK
if [ "$network" -eq 2 ]; then
    # Public, explicit registry pull. Record resolved immutable digest; never run this mutable tag.
    phase=registry_pull
    docker pull --platform linux/arm64 docker.io/library/busybox:1.37.0
    test "$(docker image inspect --format '{{.Architecture}}' docker.io/library/busybox:1.37.0)" = arm64
    digest=$(docker image inspect --format '{{index .RepoDigests 0}}' docker.io/library/busybox:1.37.0)
    printf "%s\n" "$digest" | grep -Eq "^busybox@sha256:[0-9a-f]{64}$|^docker.io/library/busybox@sha256:[0-9a-f]{64}$"
    echo "NEXUS_VM_REGISTRY_DIGEST:$digest"
    echo NEXUS_VM_REGISTRY_PULL_OK
fi
# The image is made from a signed Ubuntu busybox-static package, not a
# registry pull. The container has its own namespaces and no network.
phase=docker_import
mkdir -p /tmp/nexus-image/bin
cp "$(command -v busybox)" /tmp/nexus-image/bin/busybox
tar --numeric-owner -C /tmp/nexus-image -cf - . | docker import - nexus-vm-smoke:local >/dev/null
phase=docker_run
test "$(docker run --pull=never --network none --rm nexus-vm-smoke:local /bin/busybox echo NEXUS_CONTAINER_42)" = NEXUS_CONTAINER_42
echo NEXUS_VM_DOCKER_RUN_OK
phase=compose_volume
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
if [ "$network" -eq 1 ]; then
    phase=compose_network
    mkdir -p /tmp/nexus-web/bin /tmp/nexus-web/www/cgi-bin
    cp "$(command -v busybox)" /tmp/nexus-web/bin/busybox
    ln -s busybox /tmp/nexus-web/bin/sh
    cat >/tmp/nexus-web/www/cgi-bin/probe <<'CGI'
#!/bin/sh
printf '%s\n' NEXUS_FORWARD_ACK >/tmp/nexus-forward-proof
printf 'Content-Type: text/plain\r\n\r\nNEXUS_VM_HTTP_42\n'
CGI
    chmod 755 /tmp/nexus-web/www/cgi-bin/probe
    mkdir -p /tmp/nexus-web/tmp
    tar --numeric-owner -C /tmp/nexus-web -cf - . | docker import - nexus-vm-web:local >/dev/null
    mkdir -p /tmp/nexus-network-compose
    cd /tmp/nexus-network-compose
    cat >compose.yaml <<'YAML'
services:
  web:
    image: nexus-vm-web:local
    command: ["/bin/busybox", "httpd", "-f", "-p", "8080", "-h", "/www"]
    ports: ["8080:8080"]
  client:
    image: nexus-vm-smoke:local
    command: ["/bin/busybox", "sh", "-c", "for i in $$(/bin/busybox seq 1 60); do /bin/busybox wget -qO- http://web:8080/cgi-bin/probe && exit 0; /bin/busybox sleep 1; done; exit 1"]
YAML
    docker compose -p nexus-network up -d --no-build --pull never
    client=$(docker compose -p nexus-network ps -a -q client)
    web=$(docker compose -p nexus-network ps -q web)
    test -n "$client" && test -n "$web"
    test "$(docker wait "$client")" = 0
    test "$(docker logs "$client")" = NEXUS_VM_HTTP_42
    echo NEXUS_VM_COMPOSE_NETWORK_OK
    # Remove the internal client acknowledgement. Only the external
    # loopback request after this marker can satisfy the next assertion.
    docker exec "$web" /bin/busybox rm -f /tmp/nexus-forward-proof
    phase=loopback_forward
    echo NEXUS_VM_FORWARD_READY
    i=0
    until docker exec "$web" /bin/busybox test -f /tmp/nexus-forward-proof; do
        i=$((i+1)); test "$i" -lt 180; sleep 1
    done
    test "$(docker exec "$web" /bin/busybox cat /tmp/nexus-forward-proof)" = NEXUS_FORWARD_ACK
    echo NEXUS_VM_FORWARD_OK
    docker compose -p nexus-network down --volumes
    test -z "$(docker ps -aq)"
fi
phase=daemon_shutdown
kill -TERM "$daemon"
wait "$daemon"
daemon=
kill -TERM "$containerd_pid"
wait "$containerd_pid"
containerd_pid=
echo NEXUS_VM_CONTAINERD_STOPPED
phase=persistence
next=$((previous+1))
printf "%s\n" "$next" >"$counter.new"
mv "$counter.new" "$counter"
sync
echo "NEXUS_VM_PERSIST_SAVED:$next"
echo NEXUS_VM_CLEAN_SHUTDOWN
