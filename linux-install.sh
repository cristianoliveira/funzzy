#!/usr/bin/env bash
# Funzzy Linux installer.
#
# Downloads the matching v2 release archive (funzzy-v<VERSION>-<target>.tar.gz
# — same names produced by .github/workflows/on-release-bin.yml), verifies the
# published sha256, and installs both binaries (funzzy + fzz) into PREFIX.
#
# Overridable inputs (all optional; used by scripts/linux-install-test):
#   $1 / $VERSION   release to install (default: master's Cargo.toml version)
#   $BASE           archive base URL
#   $PREFIX         install directory (default /usr/local/bin)
#   $FORCE_ARCH     pretend `uname -m` returned this (test seam)
set -euo pipefail

PREFIX="${PREFIX:-/usr/local/bin}"
BASE="${BASE:-https://github.com/cristianoliveira/funzzy/releases/download}"
VERSION="${1:-${VERSION:-}}"
if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSL https://raw.githubusercontent.com/cristianoliveira/funzzy/master/Cargo.toml \
    | grep -m1 '^version' | awk -F\" '{print $2}')"
fi
VERSION="v${VERSION#v}"

ARCH="${FORCE_ARCH:-$(uname -m)}"
case "$ARCH" in
  x86_64|amd64)  TARGET="x86_64-linux" ;;
  aarch64|arm64) TARGET="aarch64-linux" ;;
  *) echo "Unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
esac

ARCHIVE="funzzy-${VERSION}-${TARGET}.tar.gz"
URL="${BASE}/${VERSION}/${ARCHIVE}"

echo "Installing funzzy ${VERSION} (${TARGET})"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading ${URL}"
curl -fsSL -o "$TMP/$ARCHIVE" "$URL"
curl -fsSL -o "$TMP/$ARCHIVE.sha256" "${URL}.sha256"

# sha256 files record the bare archive name; verify from the download dir.
( cd "$TMP" && shasum -a 256 -c "$ARCHIVE.sha256" >/dev/null ) \
  || { echo "Checksum mismatch for $ARCHIVE — refusing to install." >&2; exit 1; }

tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
mkdir -p "$PREFIX"
install -m 0755 "$TMP/pkg/funzzy" "$PREFIX/funzzy"
install -m 0755 "$TMP/pkg/fzz" "$PREFIX/fzz"

echo "Installed $PREFIX/funzzy and $PREFIX/fzz ($VERSION)"
echo "To uninstall: rm $PREFIX/funzzy $PREFIX/fzz"
