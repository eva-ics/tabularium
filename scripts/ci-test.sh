#!/usr/bin/env bash
# Sacred integration rite (invoked by `just ci-test`):
#   1. rsync tree → root@${CI_HOST}:/opt/${CI_PROJECT} (excludes target/, .git/)
#   2. Remote: deps + `npm ci && npm run build` in ui/; then cargo build --workspace --release
#   3. Best-effort: free CI_LISTEN port (fuser/lsof) so no stray server blocks the bind
#   4. Ephemeral DB + index under /tmp; tabularium-server (release) in background, logs → $CI_TMP/server.log
#   5. pytest tests/ with TABULARIUM_TEST_URL=http://<CI_HOST>:<port> (Selenium requires 10.90.1.122 per AGENTS.md) + TABULARIUM_TB_BIN; then remote `just test`
#   6. trap EXIT/INT/TERM: kill server PID, rm $CI_TMP
#
# Env: CI_HOST (default 10.90.1.122), CI_PROJECT (default tabularium), CI_LISTEN (default 127.0.0.1:13050 — port only used; server binds 0.0.0.0).
set -euo pipefail

CI_HOST="${CI_HOST:-10.90.1.122}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CI_PROJECT="${CI_PROJECT:-tabularium}"
REMOTE_DIR="/opt/$CI_PROJECT"
CI_LISTEN="${CI_LISTEN:-127.0.0.1:13050}"

rsync -az --delete \
  --exclude=target \
  --exclude=.git \
  --exclude=ui/node_modules \
  --exclude='.DS_Store' \
  "$ROOT/" "root@${CI_HOST}:${REMOTE_DIR}/"

ssh -o ConnectTimeout=30 -o ServerAliveInterval=10 "root@${CI_HOST}" bash -s -- "$REMOTE_DIR" "$CI_LISTEN" "$CI_HOST" <<'REMOTE'
set -euo pipefail
REMOTE_DIR="$1"
CI_LISTEN="$2"
CI_HOST_IP="$3"
cd "$REMOTE_DIR"
# Node tarball lands in /usr/local; must precede distro /usr/bin (e.g. Ubuntu nodejs without npm / libnode-dev conflicts).
export PATH="/usr/local/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
. "$HOME/.cargo/env" 2>/dev/null || true

if ! command -v cargo &>/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain stable
  . "$HOME/.cargo/env"
fi

if ! command -v just &>/dev/null; then
  cargo install just --locked
fi

if ! command -v python3 &>/dev/null; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq python3 python3-venv python3-pip
fi

# Matches ui toolchain (Vite 8: Node ^20.19 || ^22.12+).
node_toolchain_ok() {
  command -v node &>/dev/null && command -v npm &>/dev/null || return 1
  node -e '
    const [maj, min] = process.versions.node.split(".").map(Number);
    const ok =
      maj > 22 ||
      (maj === 22 && min >= 12) ||
      (maj === 20 && min >= 19) ||
      maj >= 23;
    process.exit(ok ? 0 : 1);
  ' 2>/dev/null
}

if ! node_toolchain_ok; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq || true
  apt-get install -y -qq curl ca-certificates xz-utils || true
  NODE_TAR_VER="v22.14.0"
  case "$(uname -m)" in
    x86_64) NODE_ARCH=linux-x64 ;;
    aarch64) NODE_ARCH=linux-arm64 ;;
    *) echo "ci-test: unsupported uname -m for Node tarball" >&2; exit 1 ;;
  esac
  TMP_NODE="$(mktemp /tmp/node-XXXXXX.tar.xz)"
  curl -fsSL "https://nodejs.org/dist/${NODE_TAR_VER}/node-${NODE_TAR_VER}-${NODE_ARCH}.tar.xz" -o "$TMP_NODE"
  tar -xJf "$TMP_NODE" -C /usr/local --strip-components=1
  rm -f "$TMP_NODE"
fi

if ! node_toolchain_ok; then
  echo "ci-test: need Node ^20.19 or ^22.12+ (and npm) after bootstrap" >&2
  exit 1
fi

(cd "$REMOTE_DIR/ui" && rm -rf node_modules && npm ci && npm run build)

cargo build --workspace --release

TB_BIN="$REMOTE_DIR/target/release/tb"
if [[ ! -x "$TB_BIN" ]]; then
  echo "ci-test: missing release tb at $TB_BIN" >&2
  exit 1
fi
export TABULARIUM_TB_BIN="$TB_BIN"

CI_TMP="$(mktemp -d /tmp/tabularium-ci.XXXXXX)"
SERVER_PID=""

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    sleep 0.2
    if kill -0 "$SERVER_PID" 2>/dev/null; then
      kill -9 "$SERVER_PID" 2>/dev/null || true
    fi
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "${CI_TMP:-}"
}
trap cleanup EXIT INT TERM

PORT="${CI_LISTEN##*:}"
LISTEN_ALL="0.0.0.0:${PORT}"

# Free listen port if a prior rite left a stray server (best-effort).
if [[ -n "$PORT" ]]; then
  if command -v fuser &>/dev/null; then
    fuser -k "${PORT}/tcp" 2>/dev/null || true
    sleep 0.3
  elif command -v lsof &>/dev/null; then
    for p in $(lsof -t -iTCP:"${PORT}" -sTCP:LISTEN 2>/dev/null || true); do
      kill "$p" 2>/dev/null || true
    done
    sleep 0.3
  fi
fi

mkdir -p "$CI_TMP/db"
cat >"$CI_TMP/config.toml" <<EOF
[server]
listen = "$LISTEN_ALL"
database_path = "$CI_TMP/db/tabularium.db"
index_dir = "$CI_TMP/index"
workers = 1
EOF

python3 -m venv "$CI_TMP/venv"
"$CI_TMP/venv/bin/pip" install -q -r "$REMOTE_DIR/requirements-dev.txt"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq chromium-browser chromium-chromedriver 2>/dev/null || true

export TABULARIUM_CI=1

RUST_LOG="${RUST_LOG:-warn}" nohup "$REMOTE_DIR/target/release/tabularium-server" \
  "$CI_TMP/config.toml" >"$CI_TMP/server.log" 2>&1 &
SERVER_PID=$!

# Pytest + Selenium must use CI host IP, never loopback (AGENTS.md).
BASE_URL="http://${CI_HOST_IP}:${PORT}"

for _ in $(seq 1 90); do
  if python3 -c "import urllib.request; urllib.request.urlopen('${BASE_URL}/api/doc', timeout=2)" 2>/dev/null; then
    break
  fi
  sleep 0.2
done

if ! python3 -c "import urllib.request; urllib.request.urlopen('${BASE_URL}/api/doc', timeout=2)" 2>/dev/null; then
  echo "tabularium-server failed to become ready; log tail:" >&2
  tail -80 "$CI_TMP/server.log" >&2 || true
  exit 1
fi

export TABULARIUM_TEST_URL="$BASE_URL"
export TABULARIUM_URL="$BASE_URL"
# Release CLI on PATH for tests that only check `which tb`.
export PATH="$REMOTE_DIR/target/release:$PATH"
"$CI_TMP/venv/bin/python" -m pytest "$REMOTE_DIR/tests" -v --tb=short

cd "$REMOTE_DIR"
just test
REMOTE

echo "ci-test: the rite is complete; the Omnissiah approves."
