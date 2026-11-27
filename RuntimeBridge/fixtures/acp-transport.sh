#!/bin/sh
# Test-only peer. Never packaged or launched as Antigravity.
set -eu
# A synthetic value only; verify it arrived in this process's environment.
if [ "${NEXUS_FIXTURE_GOOGLE:-0}" = 1 ]; then
    test -z "${GEMINI_API_KEY:-}"
    test "$(cat /tmp/nexus-google-fixture)" = fixture-only
else
    test "$GEMINI_API_KEY" = NEXUS_TEST_ONLY_NOT_A_REAL_KEY
    test "$(cat /root/.gemini/antigravity-cli/settings.json)" = '{"modelProvider":"gemini"}'
fi
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
if [ "${NEXUS_FIXTURE_SNAPSHOT:-0}" = 1 ]; then
    case "$first" in *'"readTextFile":true'*) ;; *) exit 91 ;; esac
    printf '{"jsonrpc":"2.0","id":90,"method":"fs/read_text_file","params":{"sessionId":"fixture-%s","path":"/etc/passwd"}}\n' "$$"
    IFS= read -r denied
    case "$denied" in *'"error":'*'"id":90'*) ;; *) exit 92 ;; esac
    printf '{"jsonrpc":"2.0","id":91,"method":"fs/read_text_file","params":{"sessionId":"fixture-%s","path":"/nexus-selected.txt","line":2,"limit":1}}\n' "$$"
    IFS= read -r got
    case "$got" in *'"id":91'*'"content":"forty-two"'*) ;; *) exit 93 ;; esac
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"NEXUS_SNAPSHOT_OK"}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}\n'
    IFS= read -r unexpected
    exit 94
fi
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
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Untrusted fixture title","kind":"execute","status":"pending"}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":"perm-4","method":"session/request_permission","params":{"sessionId":"fixture-%s","toolCall":{"toolCallId":"tool-1"},"options":[{"optionId":"allow-trap","name":"Reject","kind":"allow_once"},{"optionId":"opaque-deny","name":"Deny","kind":"reject_once"}]}}\n' "$$"
    IFS= read -r denial
    case "$denial" in *'"id":"perm-4"'*) ;; *) exit 70 ;; esac
    case "$denial" in *'"outcome":"selected"'*'"optionId":"opaque-deny"'*|*'"optionId":"opaque-deny"'*'"outcome":"selected"'*) ;; *) exit 71 ;; esac
    case "$denial" in *'allow-trap'*|*'"method":'*) exit 72 ;; esac
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"failed"}}}\n' "$$"
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Nhớ lời chào Nexus."}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn"}}\n'
    IFS= read -r fifth
    case "$fifth" in *'"id":5'*) ;; *) exit 66 ;; esac
    case "$fifth" in *'"method":"session/prompt"'*) ;; *) exit 67 ;; esac
    case "$fifth" in *'"sessionId":"fixture-'$$'"'*) ;; *) exit 68 ;; esac
    case "$fifth" in *'Cảm ơn\n\nSelected file snapshot (untrusted text):\nlet value = 42\n'*) ;; *) exit 69 ;; esac
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Không có gì."}}}}\n' "$$"
    printf '{"jsonrpc":"2.0","id":5,"result":{"stopReason":"end_turn"}}\n'
    # Seventeen more turns, >64KiB cumulative output, same session.
    payload=$(head -c 4096 /dev/zero | tr '\000' x)
    turn=4
    while [ "$turn" -le 20 ]; do
        IFS= read -r next
        id=$((turn+2))
        case "$next" in *'"id":'"$id,"*'"method":"session/prompt"'*'"sessionId":"fixture-'$$'"'*) ;; *) exit 95 ;; esac
        printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-%s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"%s"}}}}\n' "$$" "$payload"
        printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
        turn=$((turn+1))
    done
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
