#!/bin/sh
set -eu
read_frame() {
    IFS= read -r header
    len=$(printf '%s' "$header" | tr -d '\r' | cut -d ' ' -f 2)
    IFS= read -r blank
    test "$(printf '%s' "$blank" | tr -d '\r')" = ""
    body=$(dd bs=1 count="$len" 2>/dev/null)
}
send_frame() { printf 'Content-Length: %s\r\n\r\n%s' "${#1}" "$1"; }
read_frame
case "$body" in *'"id":1'*'"method":"initialize"'*) ;; *) exit 10;; esac
send_frame '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"positionEncoding":"utf-16","completionProvider":{}}}}'
read_frame
case "$body" in *'"method":"initialized"'*) ;; *) exit 11;; esac
read_frame
case "$body" in *'"method":"textDocument/didOpen"'*'"text":"x = 1\n"'*'"uri":"file:///tmp/nexus-lsp.py"'*) ;; *) exit 12;; esac
shutdown_id=2
case "$NEXUS_LSP_PHASE" in
completion*)
    read_frame
    case "$body" in *'"id":2'*'"method":"textDocument/completion"'*'"character":5'*'"line":0'*) ;; *) exit 20;; esac
    if [ "$NEXUS_LSP_PHASE" = completionInvalid ]; then
        send_frame '{"jsonrpc":"2.0","id":2,"result":{"itemDefaults":{},"items":[]}}'
    else
        send_frame '{"jsonrpc":"2.0","id":2,"result":[{"label":"add","textEdit":{"range":{"start":{"line":0,"character":5},"end":{"line":0,"character":5}},"newText":" + 2"}},{"label":"snippet","insertTextFormat":2,"insertText":"$1"},{"label":"command","command":{"command":"run"}},{"label":"bad","textEdit":{"range":{"start":{"line":0,"character":99},"end":{"line":0,"character":99}},"newText":"bad"}}]}'
    fi
    shutdown_id=3;;
*)
uri=file:///tmp/nexus-lsp.py
end=1
if [ "$NEXUS_LSP_PHASE" = invalidUri ]; then uri=file:///etc/passwd; fi
if [ "$NEXUS_LSP_PHASE" = invalidRange ]; then end=999; fi
payload=$(printf '{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"%s","version":1,"diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":%s}},"message":"fixture diagnostic","severity":2}]}}' "$uri" "$end")
send_frame "$payload"
;;
esac
read_frame
case "$body" in *'"method":"textDocument/didClose"'*) ;; *) exit 13;; esac
read_frame
case "$body" in *'"id":'"$shutdown_id"*'"method":"shutdown"'*) ;; *) exit 14;; esac
send_frame "$(printf '{"jsonrpc":"2.0","id":%s,"result":null}' "$shutdown_id")"
read_frame
case "$body" in *'"method":"exit"'*) exit 0;; *) exit 15;; esac
