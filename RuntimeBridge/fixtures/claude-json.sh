#!/bin/sh
set -eu
case "${NEXUS_CLAUDE_FIXTURE_PHASE:-}" in
conversation)
    id=550e8400-e29b-41d4-a716-446655440000
    resume=
    safe=0
    tools=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
        --safe-mode) safe=1; shift;;
        --tools) test "$2" = ""; tools=1; shift 2;;
        --resume) resume=$2; shift 2;;
        --) shift; break;;
        *) shift;;
        esac
    done
    test "$safe" = 1
    test "$tools" = 1
    test "$#" = 1
    case "$1" in
    'First fixture')
        if [ "$NEXUS_CONVERSATION_PHASE" = resume ]; then
            test "$resume" = "$id"
            test "$(cat /root/nexus-claude-session-fixture)" = "$id"
        else
            test -z "$resume"
            printf '%s\n' "$id" >/root/nexus-claude-session-fixture
        fi
        reply="First reply";;
    'Second fixture')
        test "$resume" = "$id"
        test "$(cat /root/nexus-claude-session-fixture)" = "$id"
        reply="Second reply";;
    *) exit 7;;
    esac
    printf '{"type":"result","subtype":"success","is_error":false,"session_id":"%s","result":"%s"}\n' "$id" "$reply"
    exit 0;;
signedIn) printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","email":"fixture-private"}'; exit 0;;
signedOut) printf '%s\n' '{"loggedIn":false,"authMethod":"none"}'; exit 1;;
success) printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"Hello Claude"}'; exit 0;;
error) printf '%s\n' '{"type":"result","subtype":"error","is_error":true,"result":"fixture-private"}'; exit 1;;
*) exit 2;;
esac
