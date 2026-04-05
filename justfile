# Tabularium — run with: just test, just fmt, etc.

default:
    just --list

# Vite → ui/dist, embedded by tabularium-server via include_dir!. Run before any server compile.
build-ui:
    cd ui && npm ci && npm run build

# Uses `config.toml` if present, else `config.toml.example` (fresh clones work without copying).
run: build-ui
    bash -ec 'cfg=config.toml; [[ -f "$cfg" ]] || cfg=config.toml.example; exec cargo run -p tabularium-server --release -- "$cfg"'

tb *args:
    cargo run -p tabularium-cli --release -- {{args}}

fmt:
    cargo fmt --all

# Patch-bump [workspace.package] version, then cargo check --workspace (refresh lock/metadata).
bump:
    python3 scripts/workspace_version.py bump

# Set workspace version to X.Y.Z (digits only), then cargo check --workspace.
set-version version:
    python3 scripts/workspace_version.py set "{{version}}"

check: build-ui
    cargo check --workspace

# Library only: storage stack without `reqwest` (matches server dependency shape).
check-tabularium-db:
    cargo check -p tabularium --no-default-features --features db

# Minimal crate surface (`error`, `validation`, `resource_path` only).
check-tabularium-core:
    cargo check -p tabularium --no-default-features

# Release-mode tests (integration rite / CI parity).
test: build-ui
    cargo test --workspace --release

# Fast local iteration (debug artifacts).
test-dev: build-ui
    cargo test --workspace

# Local release-only (no pytest / no server).
test-ci: build-ui
    cargo test --workspace --release

clippy: build-ui
    clippy

# Remote integration rite: rsync → /opt/${CI_PROJECT:-tabularium}, release build, ephemeral server (logs in /tmp), pytest, `just test`, trap kills server.
# Override: CI_HOST CI_PROJECT CI_LISTEN. See scripts/ci-test.sh.
ci-test:
    bash scripts/ci-test.sh

# --- Debian packages (.deb) ---
# Build needs https://github.com/cross-rs/cross (and Docker) when not on the target Linux arch.
# Packing needs `dpkg-deb` (run on Debian/Ubuntu or a container even if cross ran elsewhere).
deb-amd64: build-ui
    cross build --target x86_64-unknown-linux-gnu --release --features mcp -p tabularium-server -p tabularium-cli
    env TARGET_DIR=target RUST_TARGET=x86_64-unknown-linux-gnu DEB_ARCH=amd64 bash make-deb/build.sh

deb-arm64: build-ui
    cross build --target aarch64-unknown-linux-gnu --release --features mcp -p tabularium-server -p tabularium-cli
    env TARGET_DIR=target RUST_TARGET=aarch64-unknown-linux-gnu DEB_ARCH=arm64 bash make-deb/build.sh

# Both architectures (sequential).
deb: deb-amd64 deb-arm64

# On x86_64 Linux: same artifacts without Docker/cross.
deb-native-amd64: build-ui
    cargo build --target x86_64-unknown-linux-gnu --release --features mcp -p tabularium-server -p tabularium-cli
    env TARGET_DIR=target RUST_TARGET=x86_64-unknown-linux-gnu DEB_ARCH=amd64 bash make-deb/build.sh
