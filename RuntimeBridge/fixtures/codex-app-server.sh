#!/bin/sh
set -eu
IFS= read -r init
case "$init" in *'"id":1'*'"method":"initialize"'*'"name":"nexus-ios"'*) ;; *) exit 10;; esac
printf '%s\n' '{"id":1,"result":{"userAgent":"fixture/0.0.1"}}'
IFS= read -r ready
test "$ready" = '{"method":"initialized"}'
IFS= read -r account
test "$account" = '{"id":2,"method":"account/read","params":{"refreshToken":false}}'
printf '%s\n' '{"method":"account/updated","params":{"authMode":null}}' '{"id":2,"result":{"account":null,"requiresOpenaiAuth":true}}'
IFS= read -r unexpected
exit 11
