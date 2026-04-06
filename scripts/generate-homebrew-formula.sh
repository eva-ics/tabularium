#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPLATE="$ROOT/scripts/homebrew-formula.rb.in"

TAG=""
TARGET=""
SUMS=""
CONFIG_FILE="$ROOT/config.toml.example"
OUT=""
GITHUB_REPO="${GITHUB_REPOSITORY:-eva-ics/tabularium}"

usage() {
  echo "Usage: $0 --tag vX.Y.Z --target TRIPLE --sums PATH [--config PATH] --out PATH"
  echo "  Environment: GITHUB_REPOSITORY=owner/repo (default: eva-ics/tabularium)"
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag)
      TAG="${2:-}"
      shift 2
      ;;
    --target)
      TARGET="${2:-}"
      shift 2
      ;;
    --sums)
      SUMS="${2:-}"
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

if [[ -z "$TAG" || -z "$TARGET" || -z "$SUMS" || -z "$OUT" ]]; then
  usage
fi

if [[ ! -f "$TEMPLATE" ]]; then
  echo "Missing template: $TEMPLATE"
  exit 1
fi

if [[ ! -f "$SUMS" ]]; then
  echo "Missing SHA256SUMS file: $SUMS"
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

TB_SHA=$(grep -E 'tb-.*\.tar\.gz' "$SUMS" | head -1 | awk '{print $1}')
SRV_SHA=$(grep -E 'tabularium-server-.*\.tar\.gz' "$SUMS" | head -1 | awk '{print $1}')
CONFIG_SHA=$(shasum -a 256 "$CONFIG_FILE" | awk '{print $1}')

if [[ -z "$TB_SHA" || -z "$SRV_SHA" ]]; then
  echo "Could not parse tb / tabularium-server sha256 from $SUMS"
  exit 1
fi

TB_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/tb-${TAG}-${TARGET}.tar.gz"
SRV_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/tabularium-server-${TAG}-${TARGET}.tar.gz"
CFG_URL="https://raw.githubusercontent.com/${GITHUB_REPO}/${TAG}/config.toml.example"

substitute() {
  sed \
    -e "s|@GITHUB_REPO@|${GITHUB_REPO}|g" \
    -e "s|@VER_NUM@|${VER_NUM}|g" \
    -e "s|@TB_URL@|${TB_URL}|g" \
    -e "s|@TB_SHA@|${TB_SHA}|g" \
    -e "s|@SRV_URL@|${SRV_URL}|g" \
    -e "s|@SRV_SHA@|${SRV_SHA}|g" \
    -e "s|@CFG_URL@|${CFG_URL}|g" \
    -e "s|@CONFIG_SHA@|${CONFIG_SHA}|g"
}

substitute < "$TEMPLATE" > "$OUT"
ruby -c "$OUT"
echo "Wrote $OUT"
