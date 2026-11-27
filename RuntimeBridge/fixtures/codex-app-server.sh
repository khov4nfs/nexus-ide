#!/bin/sh
set -eu
IFS= read -r init
case "$init" in *'"id":1'*'"method":"initialize"'*'"name":"nexus-ios"'*) ;; *) exit 10;; esac
printf '%s\n' '{"id":1,"result":{"userAgent":"fixture/0.0.1"}}'
IFS= read -r ready
test "$ready" = '{"method":"initialized"}'
IFS= read -r account
test "$account" = '{"id":2,"method":"account/read","params":{"refreshToken":false}}'
if [ -n "${NEXUS_CODEX_FIXTURE_TURN:-}" ]; then
    printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt"},"requiresOpenaiAuth":true}}'
    IFS= read -r start
    case "$start" in *'"method":"thread/start"'*'"approvalPolicy":"never"'*'"sandbox":"read-only"'*) ;; *) exit 20;; esac
    if [ "$NEXUS_CODEX_FIXTURE_TURN" = policy ]; then
        printf '%s\n' '{"id":3,"result":{"thread":{"id":"t"},"approvalPolicy":"never","sandbox":{"type":"workspaceWrite","networkAccess":false}}}'
    else
        printf '%s\n' '{"id":3,"result":{"thread":{"id":"t"},"approvalPolicy":"never","sandbox":{"type":"readOnly","networkAccess":false}}}'
        IFS= read -r turn
        case "$turn" in *'"method":"turn/start"'*'Hello fixture'*'"threadId":"t"'*) ;; *) exit 21;; esac
        printf '%s\n' '{"method":"turn/started","params":{"threadId":"t","turn":{"id":"u","status":"inProgress"}}}' '{"id":4,"result":{"turn":{"id":"u","status":"inProgress"}}}' '{"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","delta":"Hello "}}'
        if [ "$NEXUS_CODEX_FIXTURE_TURN" = mismatch ]; then
            printf '%s\n' '{"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"wrong","delta":"native"}}'
        else
            printf '%s\n' '{"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","delta":"native"}}' '{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}'
        fi
    fi
else
    printf '%s\n' '{"method":"account/updated","params":{"authMode":null}}' '{"id":2,"result":{"account":null,"requiresOpenaiAuth":true}}'
fi
IFS= read -r unexpected
exit 11
