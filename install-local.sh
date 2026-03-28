#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${CLAW_INSTALL_DIR:-/usr/local/bin}"

echo "Building claw (release)..."
cargo build --release

BINARY="target/release/claw"
if [[ ! -f "$BINARY" ]]; then
  echo "error: build failed, $BINARY not found"
  exit 1
fi

echo "Installing to ${INSTALL_DIR}/claw..."
if [[ -w "$INSTALL_DIR" ]]; then
  cp "$BINARY" "${INSTALL_DIR}/claw"
else
  sudo cp "$BINARY" "${INSTALL_DIR}/claw"
fi

echo "Done: $(claw --version 2>/dev/null || echo "installed at ${INSTALL_DIR}/claw")"
echo ""
echo "Add to Claude Code:"
echo "  claude mcp add claw -- claw mcp"
