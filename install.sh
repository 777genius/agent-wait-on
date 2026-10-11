#!/bin/sh
# Verified release acquisition; Universal Agent Plugins owns client configuration.
set -eu

VERSION=0.3.0
REPOSITORY=777genius/agent-wait-on
if [ "${1:-}" = '--help' ] || [ "${1:-}" = '-h' ]; then
  printf 'Usage: sh install.sh [agentplugins add options]\nExample: sh install.sh --target codex,claude\nOmit --target for Universal Agent Plugins client discovery.\n'
  exit 0
fi
case "$(uname -s):$(uname -m)" in
  Darwin:arm64|Darwin:aarch64) PLATFORM=darwin-arm64 ;;
  Darwin:x86_64|Darwin:amd64) PLATFORM=darwin-amd64 ;;
  Linux:x86_64|Linux:amd64) PLATFORM=linux-amd64 ;;
  Linux:arm64|Linux:aarch64) PLATFORM=linux-arm64 ;;
  *) printf 'Unsupported platform. Version 0.3.0 supports macOS and Linux; Windows support is tracked in issue #4.\n' >&2; exit 1 ;;
esac
for tool in curl unzip diff awk; do
  command -v "$tool" >/dev/null 2>&1 || { printf 'Required command missing: %s\n' "$tool" >&2; exit 1; }
done
if command -v sha256sum >/dev/null 2>&1; then
  hash_file() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  hash_file() { shasum -a 256 "$1" | awk '{print $1}'; }
else
  printf 'Required command missing: sha256sum or shasum\n' >&2; exit 1
fi
if command -v agentplugins >/dev/null 2>&1; then
  uap() { agentplugins "$@"; }
elif command -v npx >/dev/null 2>&1; then
  uap() { npx --yes universal-agent-plugins@0.1.80 "$@"; }
else
  printf 'Install Universal Agent Plugins (agentplugins), or Node.js with npx, then retry.\n' >&2; exit 1
fi

ASSET="agent-wait-on-v${VERSION}-${PLATFORM}.zip"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/agent-wait-on/releases/v${VERSION}/${PLATFORM}"
SOURCE="$CACHE/package"
URL="https://github.com/$REPOSITORY/releases/download/v$VERSION"
mkdir -p "$CACHE"
WORK=$(mktemp -d "$CACHE/.install.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
trap 'exit 1' HUP INT TERM
download() { curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 "$URL/$1" --output "$2"; }

# Cache the immutable release inputs; validate them again on every invocation.
if [ ! -f "$CACHE/SHA256SUMS" ]; then
  download SHA256SUMS "$WORK/SHA256SUMS"
  mv "$WORK/SHA256SUMS" "$CACHE/SHA256SUMS"
fi
EXPECTED=$(awk -v name="$ASSET" '$2 == name || $2 == "*" name {print tolower($1); count++} END {if (count != 1) exit 1}' "$CACHE/SHA256SUMS") || {
  printf 'Release checksums must contain exactly one entry for %s\n' "$ASSET" >&2; exit 1
}
case "$EXPECTED" in *[!a-fA-F0-9]*|'') printf 'Invalid SHA256 checksum\n' >&2; exit 1 ;; esac
[ "${#EXPECTED}" -eq 64 ] || { printf 'Invalid SHA256 checksum length\n' >&2; exit 1; }
if [ ! -f "$CACHE/$ASSET" ]; then
  download "$ASSET" "$WORK/$ASSET"
  ACTUAL=$(hash_file "$WORK/$ASSET")
  [ "$ACTUAL" = "$EXPECTED" ] || { printf 'SHA256 verification failed for %s\n' "$ASSET" >&2; exit 1; }
  mv "$WORK/$ASSET" "$CACHE/$ASSET"
fi
[ "$(hash_file "$CACHE/$ASSET")" = "$EXPECTED" ] || {
  printf 'Cached archive SHA256 verification failed: %s\n' "$CACHE/$ASSET" >&2; exit 1
}
unzip -Z1 "$CACHE/$ASSET" > "$WORK/entries"
if ! awk '/^\// || /\\/ || /(^|\/)\.\.($|\/)/ {bad=1} END {exit bad}' "$WORK/entries"; then
  printf 'Archive contains unsafe paths\n' >&2; exit 1
fi
mkdir "$WORK/package"
unzip -q "$CACHE/$ASSET" -d "$WORK/package"
[ -f "$WORK/package/plugin.json" ] && [ -f "$WORK/package/mcp.json" ] && [ -f "$WORK/package/bin/agent-wait-on" ] && [ -f "$WORK/package/bin/schema/agent-exec.schema.json" ] || {
  printf 'Release archive is missing required package files\n' >&2; exit 1
}
chmod 755 "$WORK/package/bin/agent-wait-on"
if [ -e "$SOURCE" ]; then
  diff -r "$WORK/package" "$SOURCE" >/dev/null && [ -x "$SOURCE/bin/agent-wait-on" ] || {
    printf 'Cached package differs from the verified release: %s\nPreserve it for diagnosis and move it aside before retrying.\n' "$SOURCE" >&2; exit 1
  }
else
  mv "$WORK/package" "$SOURCE"
fi
printf 'Verified agent-wait-on %s (%s). Package source: %s\n' "$VERSION" "$PLATFORM" "$SOURCE"
uap validate "$SOURCE"
uap add "$SOURCE" "$@"
