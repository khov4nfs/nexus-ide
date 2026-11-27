#!/bin/sh
# Test-only MCP peer; no real tools are executed.
set -eu
IFS= read -r init
case "$init" in *'"method":"initialize"'*'"protocolVersion":"2025-11-25"'*) ;; *) exit 81 ;; esac
printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}}\n'
IFS= read -r ack
case "$ack" in *'"method":"notifications/initialized"'*) ;; *) exit 82 ;; esac
IFS= read -r list
case "$list" in *'"method":"tools/list"'*) ;; *) exit 83 ;; esac
printf '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fixture_echo","inputSchema":{"type":"object"}}]}}\n'
IFS= read -r unexpected
exit 84
