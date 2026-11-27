#!/bin/sh
# Synthetic login, not a Google client; no network or real credential.
set -eu
printf 'https://accounts.google.com/o/oauth2/v2/auth?client_id=fixture&state=fixture\n'
IFS= read -r code
test "$code" = NEXUS_FIXTURE_AUTH_CODE
umask 077
printf 'fixture-only\n' >/tmp/nexus-google-fixture
