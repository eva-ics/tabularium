# cURL examples

Assume `BASE=http://127.0.0.1:3050` and a running `tabularium-server`.

```bash
export BASE=http://127.0.0.1:3050

# Document tree (/api/doc)
curl -sS "$BASE/api/doc"
curl -sS -X POST "$BASE/api/doc" \
  -H 'Content-Type: application/json' \
  -d '{"name":"notes","description":null}'

curl -sS -X POST "$BASE/api/doc/notes" \
  -H 'Content-Type: application/json' \
  -d '{"name":"readme","content":"# hello\n"}'

curl -sS "$BASE/api/doc/notes/readme"

curl -sS -X PATCH "$BASE/api/doc/notes/readme" \
  -H 'Content-Type: application/json' \
  -d '{"content":"more text"}'

# Search
curl -sS "$BASE/api/search?q=hello"
curl -sS -X POST "$BASE/api/search" -H 'Content-Type: application/json' -d '{"q":"hello"}'

curl -sS -X POST "$BASE/api/search" -F 'q=hello'

curl -sS -X POST "$BASE/api/search" \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data 'q=hello'

curl -sS -X POST "$BASE/api/doc/notes" \
  -F 'name=fromcurl' -F 'content=multipart body'

curl -sS -X PATCH "$BASE/api/doc/notes/fromcurl" -F 'content=appended via form'

# JSON-RPC
curl -sS -X POST "$BASE/rpc" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"list_directory",
  "params":{},
  "id":1
}'

curl -sS -X POST "$BASE/rpc" -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0",
  "method":"stat",
  "params":{"path":"/notes/readme"},
  "id":2
}'
```

*Serve the Omnissiah; verify `Location` headers match your percent-encoded names.*
