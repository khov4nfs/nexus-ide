#!/bin/sh
set -eu
IFS= read -r init
case "$init" in *'"method":"initialize"'*) ;; *) exit 1;; esac
if [ "$NEXUS_ADAPTER_PHASE" = unsupportedResume ]; then
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"Synthetic ACP adapter","version":"test"}}}'
    IFS= read -r unexpected
    exit 8
fi
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true},"agentInfo":{"name":"Synthetic ACP adapter","version":"test"}}}'
IFS= read -r new
if [ "$NEXUS_ADAPTER_PHASE" = resume ]; then
    test "$(cat /root/nexus-adapter-fixture-session)" = adapter-fixture
    case "$new" in *'"method":"session/load"'*'"sessionId":"adapter-fixture"'*) ;; *) exit 9;; esac
    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"adapter-fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Old replay withheld"}}}}' '{"jsonrpc":"2.0","id":2,"result":{}}'
else
    case "$new" in *'"method":"session/new"'*'"mcpServers":[]'*) ;; *) exit 2;; esac
    printf '%s\n' adapter-fixture >/root/nexus-adapter-fixture-session
    printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"adapter-fixture"}}'
fi
IFS= read -r first
case "$first" in *'"id":3'*'First adapter prompt'*'"sessionId":"adapter-fixture"'*) ;; *) exit 3;; esac
if [ "$NEXUS_ADAPTER_PHASE" = unexpectedCallback ]; then
    printf '%s\n' '{"jsonrpc":"2.0","id":"bad","method":"fs/read_text_file","params":{"sessionId":"adapter-fixture","path":"/native-secret"}}'
    IFS= read -r unexpected
    exit 4
fi
printf '%s\n' '{"jsonrpc":"2.0","id":"perm","method":"session/request_permission","params":{"sessionId":"adapter-fixture","toolCall":{"toolCallId":"test"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"},{"optionId":"deny","name":"Deny","kind":"reject_once"}]}}'
IFS= read -r reply
case "$reply" in *'"optionId":"deny"'*) ;; *) exit 5;; esac
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"adapter-fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"First adapter reply"}}}}' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
IFS= read -r second
case "$second" in *'"id":4'*'Second adapter prompt'*'"sessionId":"adapter-fixture"'*) ;; *) exit 6;; esac
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"adapter-fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Second adapter reply"}}}}' '{"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn"}}'
IFS= read -r unexpected
exit 7
