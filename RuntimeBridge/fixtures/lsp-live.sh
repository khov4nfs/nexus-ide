#!/bin/sh
set -eu
read_frame() {
    IFS= read -r header
    len=$(printf '%s' "$header" | tr -d '\r' | cut -d ' ' -f 2)
    IFS= read -r blank
    body=$(dd bs=1 count="$len" 2>/dev/null)
}
send_frame() { printf 'Content-Length: %s\r\n\r\n%s' "${#1}" "$1"; }
diag() { send_frame "$(printf '{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///tmp/nexus-lsp.py","version":%s,"diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"%s"}]}}' "$1" "$2")"; }
read_frame
case "$body" in *'"method":"initialize"'*) ;; *) exit 1;; esac
send_frame '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"positionEncoding":"utf-16","textDocumentSync":{"openClose":true,"change":1}}}}'
read_frame
case "$body" in *'"method":"initialized"'*) ;; *) exit 2;; esac
read_frame
case "$body" in *'"method":"textDocument/didOpen"'*'"text":"x = 1\n"'*'"version":1'*) ;; *) exit 3;; esac
diag 1 'version one'
read_frame
case "$body" in *'"text":"x = 2\n"'*'"method":"textDocument/didChange"'*'"version":2'*) ;; *) exit 4;; esac
diag 1 'stale ignored'
if [ "$NEXUS_LIVE_LSP_PHASE" = future ]; then diag 3 'wrong version'; IFS= read -r unexpected; exit 5; fi
diag 2 'version two'
read_frame
case "$body" in *'"method":"textDocument/didClose"'*) ;; *) exit 6;; esac
read_frame
case "$body" in *'"id":2'*'"method":"shutdown"'*) ;; *) exit 7;; esac
send_frame '{"jsonrpc":"2.0","id":2,"result":null}'
read_frame
case "$body" in *'"method":"exit"'*) exit 0;; *) exit 8;; esac
