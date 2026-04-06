#!/usr/bin/env bash
set -euo pipefail

tap="eva-ics/tabularium"
tap_repo="$(brew --repository "$tap" 2>/dev/null || true)"
tmp_dir="$(mktemp -d)"
tar_path="$tmp_dir/tabularium.tar.gz"
sha=""
version=""

if [[ -z "$tap_repo" || ! -d "$tap_repo" ]]; then
  brew tap-new "$tap" >/dev/null
  tap_repo="$(brew --repository "$tap")"
fi

formula_dir="$tap_repo/Formula"
formula_path="$formula_dir/tabularium.rb"

mkdir -p "$formula_dir"
cp "./Formula/tabularium.rb" "$formula_path"

tar --exclude=.git --exclude=target --exclude=ui/node_modules -czf "$tar_path" .
sha="$(shasum -a 256 "$tar_path" | awk '{print $1}')"
version="$(awk -F'"' '/^version = "/ {print $2; exit}' Cargo.toml)"
if [[ -z "$version" ]]; then
  echo "Failed to read version from Cargo.toml"
  exit 1
fi

tar_url="file://$(cd "$(dirname "$tar_path")" && pwd)/$(basename "$tar_path")"
tmp_formula="$formula_path.tmp"
awk -v tar_url="$tar_url" -v sha="$sha" -v version="$version" '
{
  gsub("__TABULARIUM_TARBALL__", tar_url)
  gsub("__TABULARIUM_SHA256__", sha)
  gsub("__TABULARIUM_VERSION__", version)
  print
}
' "$formula_path" > "$tmp_formula"
mv "$tmp_formula" "$formula_path"

if brew list --formula "$tap/tabularium" >/dev/null 2>&1; then
  brew reinstall "$tap/tabularium"
else
  brew install "$tap/tabularium"
fi

brew services start tabularium