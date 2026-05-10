# Authentication and authorization

Tabularium now has built-in stage-1 authentication and authorization. The old
"put it behind something else and hope" rite is still useful, but it is no
longer the whole story.

## Surfaces and switches

`[server].authenticate = true` protects the main HTTP surface:

- web UI
- REST API under `/api`
- JSON-RPC at `POST /rpc`
- document WebSocket at `/ws`

Two health probes stay open even when server auth is enabled:

- `GET /api/test`
- JSON-RPC `test`

`[mcp].authenticate = true` is a separate switch for the streamable HTTP MCP
endpoint. It uses the same credential rules as `/rpc`, but you can keep MCP
open or closed independently from the main HTTP server.

## Supported auth modes

Tabularium supports two built-in credential paths:

1. Pre-shared keys (PSKs) mapped to ACLs
2. Optional signed assertion tokens from a trusted upstream AAA/WAF via
   `[oidc]`

Tabularium still does **not** provide:

- user accounts
- passwords
- sessions
- OAuth login redirects
- full IAM / RBAC modeling

The model is intentionally smaller: one request resolves to one effective
principal, then ACL checks happen on paths.

## PSK + ACL model

When auth is enabled and no assertion header is present, the caller must send:

```http
X-Auth-Key: <psk>
```

CLI shorthand:

```bash
tb -u http://127.0.0.1:3050 -k YOUR_PSK whoami
```

Each PSK belongs to exactly one ACL. One ACL may have many PSKs.

ACL body JSON:

```json
{
  "admin": false,
  "allow": {
    "read": ["/docs/*"],
    "write": ["/docs/meeting.md"]
  },
  "deny": {
    "read": [],
    "write": ["/docs/secret/*"]
  }
}
```

Rules:

- default deny
- `deny` overrides `allow`
- `admin: true` bypasses ACL checks, but not normal request validation
- patterns are **absolute** paths (`/` root, `/foo/bar` file or directory, `/vault/*` subtree)
- `/path/to/file` matches exactly that entry
- `/path/to/dir/*` matches strict descendants only, not `/path/to/dir` itself
- listing a directory without direct read on that path still returns **filtered**
  child rows you may read
- for navigation, a directory row may still be visible when an allow rule reaches
  strictly inside it — e.g. `/test/*` surfaces `/test` in its parent listing, and
  `/vault/deep/*` surfaces both `/vault` and `/vault/deep`

Stage-1 tradeoff, documented rather than hidden:

- PSKs are stored plaintext in the database
- `psk_list` returns plaintext keys to admin callers

That is intentional for the current operator model. If you need a more pious
threat posture, put Tabularium behind a stronger control plane or use the
upstream assertion flow below.

## Upstream signed assertion flow (`[oidc]`)

`[oidc]` is not a generic public bearer-token feature. It is a
trusted-upstream assertion flow:

- your WAF / AAA layer authenticates the caller
- that upstream emits a canonical signed assertion
- Tabularium validates the assertion locally against JWK/JWKS

Canonical config shape:

```toml
[oidc]
key = "/etc/tabularium/oidc.jwk"
# header = "X-JWT-Assertion"
# groups_field = "groups"
# refresh = 3600
# retry = 10
# timeout = 10
# group_name_prefix = "tb_"
```

Field semantics:

- `key` is required when `[oidc]` is present; startup fails if it is empty
- `key` may be a local JWKS file path or an `http(s)` URL
- `header` defaults to `X-JWT-Assertion`
- `groups_field` defaults to `groups`
- `refresh` is the normal JWKS refresh interval in seconds
- `retry` is the retry interval after a failed refresh
- `timeout` is the JWKS HTTP fetch timeout in seconds
- `group_name_prefix` filters token groups before ACL lookup; the prefix is
  removed from matched groups

Verification and precedence:

- if the configured assertion header is absent, Tabularium falls back to
  `X-Auth-Key`
- if the assertion header is present but empty or invalid, the request fails
  closed
- if the assertion header is present and non-empty, Tabularium verifies the JWT
  and does **not** fall back to PSK on failure
- signature verification is mandatory
- `exp` is required
- `nbf` is validated when present
- `iss` / `aud` are **not** configurable in the canonical format and are not
  enforced by Tabularium

JWKS refresh behavior:

- Tabularium loads JWKS at startup; unreachable or invalid JWKS aborts startup
- after startup, failed refreshes retry forever at `retry` interval
- the last good JWKS stays cached until a later refresh succeeds

## JWT groups to ACLs

JWT claims do not carry raw permissions. They resolve to local ACL names.

Process:

1. Read the configured `groups_field`
2. Optionally filter groups by `group_name_prefix`
3. Trim the prefix from matched groups
4. Look up local ACLs with those names
5. Merge all matched ACLs into one effective principal

Merge semantics:

- allows are unioned
- denies are unioned
- any matched `admin: true` ACL makes the merged principal admin

This means token order is not authorization policy. "First matching group wins"
was rejected and duly buried.

## `whoami`, ACL admin, and recovery

Any valid authenticated caller may use `whoami`:

- REST: `GET /api/whoami`
- JSON-RPC: `whoami`
- MCP: `whoami`

It returns the resolved ACL name, admin flag, and effective allow/deny rules.

ACL and PSK management methods are admin-only when auth is enabled:

- `acl_list`
- `acl_get`
- `acl_put`
- `acl_destroy`
- `psk_list`
- `psk_create`
- `psk_destroy`

When auth is disabled, those management methods are open.

If operators delete the last admin ACL, recovery is intentionally blunt:

1. disable auth in `config.toml`
2. restart the server
3. recreate an admin ACL and PSK
4. re-enable auth

## HTTP and JSON-RPC failure semantics

REST / web / WebSocket / MCP HTTP:

- missing or unknown credential: `401 Unauthorized`
- known credential but ACL denies the action: `403 Forbidden`

JSON-RPC:

- missing or unknown credential: `-32001`
- known credential but ACL denies the action: `-32004`

List and search results are filtered before they are returned. Direct path
operations are rejected as soon as the target path is known. For directory
listings, Tabularium does not require direct read access on the listed parent
directory when a narrower ACL still exposes some children beneath it.

## Recommended deployment stance

Even with built-in auth, the sensible deployment shape is still:

- keep Tabularium on a private network or trusted host
- terminate TLS at your proxy or gateway
- expose `X-JWT-Assertion` only in deployments where a trusted upstream mints it
- use `X-Auth-Key` for direct operator/API access when that simpler model fits
- prefer `tb -k` for PSKs and `TB_HEADERS` / `--header` for custom assertion
  headers

If you are exposing a public-facing service, the external control plane should
still do the heavy lifting. Tabularium's built-in auth is real, but it is not a
substitute for perimeter sanity.
