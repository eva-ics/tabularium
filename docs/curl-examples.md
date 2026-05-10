# cURL examples

Assume `BASE=http://127.0.0.1:3050` and a running `tabularium-server`.

```bash
export BASE=http://127.0.0.1:3050
export PSK=replace-me

# If [server].authenticate=true, add:
AUTH=(-H "X-Auth-Key: $PSK")

# Document tree (/api/doc)
curl -sS "${AUTH[@]}" "$BASE/api/doc"
curl -sS -X POST "$BASE/api/doc" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"name":"notes","description":null}'

curl -sS -X POST "$BASE/api/doc/notes" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"name":"readme","content":"# hello\n"}'

curl -sS "${AUTH[@]}" "$BASE/api/doc/notes/readme"

curl -sS -X PATCH "$BASE/api/doc/notes/readme" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"content":"more text"}'

# Search
curl -sS "${AUTH[@]}" "$BASE/api/search?q=hello"
curl -sS -X POST "$BASE/api/search" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{"q":"hello"}'

curl -sS -X POST "$BASE/api/search" "${AUTH[@]}" -F 'q=hello'

curl -sS -X POST "$BASE/api/search" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data 'q=hello'

curl -sS -X POST "$BASE/api/doc/notes" \
  "${AUTH[@]}" \
  -F 'name=fromcurl' -F 'content=multipart body'

curl -sS -X PATCH "$BASE/api/doc/notes/fromcurl" "${AUTH[@]}" -F 'content=appended via form'

# Identity view for the current caller
curl -sS "${AUTH[@]}" "$BASE/api/whoami"

# JSON-RPC
curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"list_directory",
  "params":{},
  "id":1
}'

curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"stat",
  "params":{"path":"/notes/readme"},
  "id":2
}'

# Safe-by-default put_document (create-only): existing target -> -32002 Duplicate.
curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"put_document",
  "params":{"path":"/notes/new_scroll","content":"# fresh\n"},
  "id":3
}'

# Deliberate overwrite: pass force=true.
curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"put_document",
  "params":{"path":"/notes/new_scroll","content":"# replaced\n","force":true},
  "id":4
}'

# Same guard for append_document (force=true appends to existing; missing target always creates).
curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"append_document",
  "params":{"path":"/notes/new_scroll","content":"\nmore","force":true},
  "id":5
}'

# Admin-only ACL / PSK management
curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"acl_put",
  "params":{
    "name":"readers",
    "body":"{\"admin\":false,\"allow\":{\"read\":[\"/docs/*\"],\"write\":[]},\"deny\":{\"read\":[],\"write\":[]}}"
  },
  "id":6
}'

curl -sS -X POST "$BASE/rpc" "${AUTH[@]}" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"psk_create",
  "params":{"name":"reader-cli","acl_name":"readers"},
  "id":7
}'

# Trusted-upstream assertion example (only if [oidc] is configured)
curl -sS "$BASE/api/whoami" \
  -H 'X-JWT-Assertion: YOUR_SIGNED_ASSERTION'
```

_Serve the Omnissiah; verify `Location` headers match your percent-encoded names._
