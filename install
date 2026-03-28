#!/bin/sh
set -e

REPO="LeonTing1010/claw"
INSTALL_DIR="${CLAW_INSTALL_DIR:-/usr/local/bin}"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) OS_TARGET="apple-darwin" ;;
  Linux)  OS_TARGET="unknown-linux-gnu" ;;
  *)      echo "error: unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64)  ARCH_TARGET="x86_64" ;;
  arm64|aarch64) ARCH_TARGET="aarch64" ;;
  *)             echo "error: unsupported architecture: $ARCH"; exit 1 ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"
TARBALL="claw-${TARGET}.tar.gz"

# Get latest release URL
RELEASE_URL="https://github.com/${REPO}/releases/latest/download/${TARBALL}"

echo "Installing claw (${TARGET})..."

# Download and extract
TMP="$(mktemp -d)"
curl -fsSL "$RELEASE_URL" -o "${TMP}/${TARBALL}"
tar -xzf "${TMP}/${TARBALL}" -C "$TMP"

# Install
if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP}/claw" "${INSTALL_DIR}/claw"
else
  sudo mv "${TMP}/claw" "${INSTALL_DIR}/claw"
fi

rm -rf "$TMP"

echo "claw installed to ${INSTALL_DIR}/claw"
echo ""
echo "Add to Claude Code:"
echo "  claude mcp add claw -- claw mcp"
