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
if [ "${NEXUS_FIXTURE_CALL:-0}" = 1 ]; then
    IFS= read -r call
    case "$call" in *'"method":"tools/call"'*'"arguments":{"value":42}'*'"name":"fixture_echo"'*) ;; *) exit 85 ;; esac
    printf '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"NEXUS_MCP_42"}],"isError":false}}\n'
fi
IFS= read -r unexpected
exit 84
