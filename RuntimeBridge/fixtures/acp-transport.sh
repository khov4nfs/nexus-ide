#!/bin/sh
# Test-only peer. Never packaged or launched as Antigravity.
set -eu
# A synthetic value only; verify it arrived in this process's environment.
test "$GEMINI_API_KEY" = NEXUS_TEST_ONLY_NOT_A_REAL_KEY
test "$(cat /root/.gemini/antigravity-cli/settings.json)" = '{"modelProvider":"gemini"}'
IFS= read -r first
case "$first" in *'"method":"initialize"'*) ;; *) exit 51 ;; esac
printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"Nexus Transport Fixture","version":"test"}}}\n'
printf 'fixture diagnostic, not ACP stdout\n' >&2
IFS= read -r second
case "$second" in *'"method":"session/new"'*) ;; *) exit 52 ;; esac
printf '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"fixture-%s"}}\n' "$$"
# Keep same process alive until the host explicitly closes it.
IFS= read -r unexpected
exit 53
