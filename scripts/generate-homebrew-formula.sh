#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPLATE="$ROOT/scripts/homebrew-formula.rb.in"

TAG=""
MAC_TARGET=""
MAC_SUMS=""
LINUX_TARGET=""
LINUX_SUMS=""
CONFIG_FILE="$ROOT/config.toml.example"
OUT=""
GITHUB_REPO="${GITHUB_REPOSITORY:-velortechnologies/tabularium}"

usage() {
  echo "Usage: $0 --tag vX.Y.Z --mac-target TRIPLE --mac-sums PATH --linux-target TRIPLE --linux-sums PATH [--config PATH] --out PATH"
  echo "  Environment: GITHUB_REPOSITORY=owner/repo (default: velortechnologies/tabularium)"
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag)
      TAG="${2:-}"
      shift 2
      ;;
    --mac-target)
      MAC_TARGET="${2:-}"
      shift 2
      ;;
    --mac-sums)
      MAC_SUMS="${2:-}"
      shift 2
      ;;
    --linux-target)
      LINUX_TARGET="${2:-}"
      shift 2
      ;;
    --linux-sums)
      LINUX_SUMS="${2:-}"
      shift 2
      ;;
    --config)
      CONFIG_FILE="${2:-}"
      shift 2
      ;;
    --out)
      OUT="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      echo "Unknown option: $1"
      usage
      ;;
  esac
done

if [[ -z "$TAG" || -z "$MAC_TARGET" || -z "$MAC_SUMS" || -z "$LINUX_TARGET" || -z "$LINUX_SUMS" || -z "$OUT" ]]; then
  usage
fi

if [[ ! -f "$TEMPLATE" ]]; then
  echo "Missing template: $TEMPLATE"
  exit 1
fi

if [[ ! -f "$MAC_SUMS" ]]; then
  echo "Missing macOS SHA256SUMS file: $MAC_SUMS"
  exit 1
fi

if [[ ! -f "$LINUX_SUMS" ]]; then
  echo "Missing Linux SHA256SUMS file: $LINUX_SUMS"
  exit 1
fi

if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "Missing config file: $CONFIG_FILE"
  exit 1
fi

if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Tag must look like vX.Y.Z, got: $TAG"
  exit 1
fi

VER_NUM="${TAG#v}"

MAC_TB_SHA=$(grep -E "tb-.*${MAC_TARGET}\.tar\.gz" "$MAC_SUMS" | awk '{print $1}')
MAC_SRV_SHA=$(grep -E "tabularium-server-.*${MAC_TARGET}\.tar\.gz" "$MAC_SUMS" | awk '{print $1}')
LINUX_TB_SHA=$(grep -E "tb-.*${LINUX_TARGET}\.tar\.gz" "$LINUX_SUMS" | awk '{print $1}')
LINUX_SRV_SHA=$(grep -E "tabularium-server-.*${LINUX_TARGET}\.tar\.gz" "$LINUX_SUMS" | awk '{print $1}')
CONFIG_SHA=$(shasum -a 256 "$CONFIG_FILE" | awk '{print $1}')

if [[ -z "$MAC_TB_SHA" || -z "$MAC_SRV_SHA" || -z "$LINUX_TB_SHA" || -z "$LINUX_SRV_SHA" ]]; then
  echo "Could not parse tb / tabularium-server sha256 from checksum files"
  exit 1
fi

MAC_TB_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/tb-${TAG}-${MAC_TARGET}.tar.gz"
MAC_SRV_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/tabularium-server-${TAG}-${MAC_TARGET}.tar.gz"
LINUX_TB_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/tb-${TAG}-${LINUX_TARGET}.tar.gz"
LINUX_SRV_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/tabularium-server-${TAG}-${LINUX_TARGET}.tar.gz"
CFG_URL="https://raw.githubusercontent.com/${GITHUB_REPO}/${TAG}/config.toml.example"

substitute() {
  sed \
    -e "s|@GITHUB_REPO@|${GITHUB_REPO}|g" \
    -e "s|@VER_NUM@|${VER_NUM}|g" \
    -e "s|@MAC_TB_URL@|${MAC_TB_URL}|g" \
    -e "s|@MAC_TB_SHA@|${MAC_TB_SHA}|g" \
    -e "s|@MAC_SRV_URL@|${MAC_SRV_URL}|g" \
    -e "s|@MAC_SRV_SHA@|${MAC_SRV_SHA}|g" \
    -e "s|@LINUX_TB_URL@|${LINUX_TB_URL}|g" \
    -e "s|@LINUX_TB_SHA@|${LINUX_TB_SHA}|g" \
    -e "s|@LINUX_SRV_URL@|${LINUX_SRV_URL}|g" \
    -e "s|@LINUX_SRV_SHA@|${LINUX_SRV_SHA}|g" \
    -e "s|@CFG_URL@|${CFG_URL}|g" \
    -e "s|@CONFIG_SHA@|${CONFIG_SHA}|g"
}

substitute < "$TEMPLATE" > "$OUT"
ruby -c "$OUT"
echo "Wrote $OUT"
