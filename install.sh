#!/usr/bin/env bash
# Install gclaw — download the latest release binary from GitHub.
#
#   curl -fsSL https://raw.githubusercontent.com/bczaicki/gclaw/main/install.sh | bash
#
# Options (env vars):
#   GCLAW_VERSION   — specific version tag (default: latest)
#   GCLAW_INSTALL   — install directory (default: ~/.local/bin)

set -euo pipefail

REPO="bczaicki/gclaw"
INSTALL_DIR="${GCLAW_INSTALL:-$HOME/.local/bin}"

# --- Detect platform ---

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64)  arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *)             echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

TARGET="${arch}-${os}"

# --- Resolve version ---

if [ -n "${GCLAW_VERSION:-}" ]; then
  VERSION="$GCLAW_VERSION"
else
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)"
  if [ -z "$VERSION" ]; then
    echo "Failed to fetch latest release version."
    exit 1
  fi
fi

ARCHIVE="gclaw-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"

echo "Installing gclaw ${VERSION} (${TARGET})"
echo "  from: ${URL}"
echo "  to:   ${INSTALL_DIR}/gclaw"

# --- Download and install ---

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "${TMPDIR}/${ARCHIVE}"
tar xzf "${TMPDIR}/${ARCHIVE}" -C "$TMPDIR"

mkdir -p "$INSTALL_DIR"
mv "${TMPDIR}/gclaw" "${INSTALL_DIR}/gclaw"
chmod +x "${INSTALL_DIR}/gclaw"

echo ""
echo "Installed gclaw to ${INSTALL_DIR}/gclaw"

# --- Check PATH ---

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo ""
  echo "Note: ${INSTALL_DIR} is not in your PATH."
  echo "Add it with:"
  echo ""
  echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  echo ""
  echo "Or add that line to your ~/.bashrc / ~/.zshrc."
fi

echo ""
echo "Run 'gclaw doctor' to verify your setup."
