#!/usr/bin/env bash
# Pack tabularium-cli and tabularium-server .deb from pre-built Linux release binaries.
# Requires: dpkg-deb (Debian/Ubuntu), GNU install.
#
# Env:
#   TARGET_DIR   — cargo target dir root (default: target)
#   RUST_TARGET  — e.g. x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
#   DEB_ARCH     — dpkg arch: amd64, arm64
#   VERSION      — optional override (default: workspace root Cargo.toml [workspace.package])
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MD="$ROOT/make-deb"

VERSION="${VERSION:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)}"
TARGET_DIR="${TARGET_DIR:-target}"
RUST_TARGET="${RUST_TARGET:-x86_64-unknown-linux-gnu}"
DEB_ARCH="${DEB_ARCH:-amd64}"

BIN_DIR="$ROOT/$TARGET_DIR/$RUST_TARGET/release"
SRV="$BIN_DIR/tabularium-server"
CLI="$BIN_DIR/tb"

if [[ ! -f "$SRV" || ! -f "$CLI" ]]; then
    echo "missing binaries under $BIN_DIR (need tabularium-server + tb); build linux --release first" >&2
    exit 1
fi

if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "dpkg-deb not found — run this step on Debian/Ubuntu (or a container)" >&2
    exit 1
fi

OUT="$MD/dist"
mkdir -p "$OUT"
rm -f "$OUT/tabularium-cli_${VERSION}_${DEB_ARCH}.deb" "$OUT/tabularium-server_${VERSION}_${DEB_ARCH}.deb"

stage_cli="$(mktemp -d)"
cleanup() { rm -rf "$stage_cli" "$stage_srv"; }
stage_srv="$(mktemp -d)"
trap cleanup EXIT

# --- tabularium-cli ---
mkdir -p "$stage_cli/usr/bin"
install -m755 "$CLI" "$stage_cli/usr/bin/tb"
mkdir -p "$stage_cli/DEBIAN"
cat >"$stage_cli/DEBIAN/control" <<EOF
Package: tabularium-cli
Version: $VERSION
Section: utils
Priority: optional
Architecture: $DEB_ARCH
Maintainer: Tabularium Maintainers <tabularium@localhost>
Depends: libc6 (>= 2.31)
Description: Tabularium CLI (tb)
 JSON-RPC client for the Tabularium librarium.
EOF

dpkg-deb --root-owner-group --build "$stage_cli" "$OUT/tabularium-cli_${VERSION}_${DEB_ARCH}.deb"

# --- tabularium-server ---
rm -rf "$stage_srv"
mkdir -p "$stage_srv/usr/sbin" "$stage_srv/etc/tabularium" "$stage_srv/lib/systemd/system" "$stage_srv/DEBIAN"
install -m755 "$SRV" "$stage_srv/usr/sbin/tabularium-server"
install -m644 "$MD/config.toml.default" "$stage_srv/etc/tabularium/config.toml.default"
install -m644 "$ROOT/systemd/tabularium-server.service" "$stage_srv/lib/systemd/system/tabularium-server.service"
install -m755 "$MD/server/postinst" "$stage_srv/DEBIAN/postinst"
install -m755 "$MD/server/prerm" "$stage_srv/DEBIAN/prerm"
install -m755 "$MD/server/postrm" "$stage_srv/DEBIAN/postrm"

cat >"$stage_srv/DEBIAN/control" <<EOF
Package: tabularium-server
Version: $VERSION
Section: net
Priority: optional
Architecture: $DEB_ARCH
Maintainer: Tabularium Maintainers <tabularium@localhost>
Depends: libc6 (>= 2.31), adduser, tabularium-cli (= $VERSION), systemd | systemd-sysv
Description: Tabularium HTTP server (REST, JSON-RPC, MCP)
 Machine-spirit gateway for the Tabularium document store.
EOF

dpkg-deb --root-owner-group --build "$stage_srv" "$OUT/tabularium-server_${VERSION}_${DEB_ARCH}.deb"

echo "built:"
echo "  $OUT/tabularium-cli_${VERSION}_${DEB_ARCH}.deb"
echo "  $OUT/tabularium-server_${VERSION}_${DEB_ARCH}.deb"
echo "Tip: refine Depends with ldd on the target host against the packaged binaries."
