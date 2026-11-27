#!/bin/sh
# Fixed offline text transfer, never evaluate input as shell or code.
set -eu
dir=$(mktemp -d /tmp/nexus-workspace.XXXXXX)
trap 'rm -rf "$dir"' EXIT
printf 'NEXUS_WORKSPACE_READY\n' >&2
IFS= read -r payload
printf '%s' "$payload" | /bin/busybox base64 -d > "$dir/input.txt"
printf '\n[Nexus guest workspace probe]\n' >> "$dir/input.txt"
/bin/cat "$dir/input.txt"
