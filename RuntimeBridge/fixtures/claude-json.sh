#!/bin/sh
set -eu
case "${NEXUS_CLAUDE_FIXTURE_PHASE:-}" in
signedIn) printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","email":"fixture-private"}'; exit 0;;
signedOut) printf '%s\n' '{"loggedIn":false,"authMethod":"none"}'; exit 1;;
success) printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"Hello Claude"}'; exit 0;;
error) printf '%s\n' '{"type":"result","subtype":"error","is_error":true,"result":"fixture-private"}'; exit 1;;
*) exit 2;;
esac
