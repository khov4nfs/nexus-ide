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
IFS= read -r third
case "$third" in *'"method":"session/prompt"'*) ;; *) exit 54 ;; esac
case "$third" in *'"sessionId":"fixture-'$$'"'*) ;; *) exit 55 ;; esac
if [ "${NEXUS_FIXTURE_CHAT:-0}" = 1 ]; then
    case "$third" in *'Xin chào Nexus'*) ;; *) exit 60 ;; esac
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Xin chào "}}}}\n' "$$"
    sleep 2
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Nexus!"}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}\n'
    # A stateful test peer: the second and third requests must arrive at
    # this same process/session, with distinct correlated IDs.
    IFS= read -r fourth
    case "$fourth" in *'"id":4'*) ;; *) exit 62 ;; esac
    case "$fourth" in *'"method":"session/prompt"'*) ;; *) exit 63 ;; esac
    case "$fourth" in *'"sessionId":"fixture-'$$'"'*) ;; *) exit 64 ;; esac
    case "$fourth" in *'Bạn nhớ lời chào không?'*) ;; *) exit 65 ;; esac
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Nhớ lời chào Nexus."}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn"}}\n'
    IFS= read -r fifth
    case "$fifth" in *'"id":5'*) ;; *) exit 66 ;; esac
    case "$fifth" in *'"method":"session/prompt"'*) ;; *) exit 67 ;; esac
    case "$fifth" in *'"sessionId":"fixture-'$$'"'*) ;; *) exit 68 ;; esac
    case "$fifth" in *'Cảm ơn'*) ;; *) exit 69 ;; esac
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Không có gì."}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":5,"result":{"stopReason":"end_turn"}}\n'
    IFS= read -r unexpected
    exit 61
fi
case "$third" in *'Reply with exactly NEXUS_ACP_OK. Do not use tools, read files, or perform actions.'*) ;; *) exit 56 ;; esac
printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"NEXUS_"}}}}\n' "$$"
if [ "${NEXUS_FIXTURE_CANCEL:-0}" = 1 ]; then
    IFS= read -r cancel
    case "$cancel" in *'"method":"session/cancel"'*) ;; *) exit 57 ;; esac
    case "$cancel" in *'"sessionId":"fixture-'$$'"'*) ;; *) exit 58 ;; esac
    case "$cancel" in *'"id":'*) exit 59 ;; esac
    printf '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}\n'
else
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ACP_OK"}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}\n'
fi
# Keep same process alive until the host explicitly closes it.
IFS= read -r unexpected
exit 53
