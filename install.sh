#!/usr/bin/env bash
set -euo pipefail

# S3A One-Line Installer for Linux and macOS
REPO="ashleycribb/s3a-storage"
INSTALL_DIR="${HOME}/.local/bin"

echo "================================================="
echo "   Stratified Simplicial Storage Architecture   "
echo "                 S3A Installer                  "
echo "================================================="

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${OS}" in
  linux)
    case "${ARCH}" in
      x86_64)
        TARGET="x86_64-unknown-linux-gnu"
        ARCHIVE_EXT="tar.gz"
        ;;
      aarch64|arm64)
        TARGET="aarch64-unknown-linux-gnu"
        ARCHIVE_EXT="tar.gz"
        ;;
      *)
        echo "Error: Unsupported Linux architecture: ${ARCH}"
        exit 1
        ;;
    esac
    ;;
  darwin)
    case "${ARCH}" in
      arm64|aarch64)
        TARGET="aarch64-apple-darwin"
        ARCHIVE_EXT="tar.gz"
        ;;
      x86_64)
        TARGET="x86_64-unknown-linux-gnu"
        echo "Warning: Native macOS x86_64 prebuilt binary not provided; falling back to cargo install or Rosetta."
        if command -v cargo >/dev/null 2>&1; then
          echo "Building from source via 'cargo install s3a-cli'..."
          cargo install s3a-cli
          exit 0
        fi
        exit 1
        ;;
      *)
        echo "Error: Unsupported macOS architecture: ${ARCH}"
        exit 1
        ;;
    esac
    ;;
  *)
    echo "Error: Unsupported operating system: ${OS}"
    echo "For Windows, run install.ps1 in PowerShell."
    exit 1
    ;;
esac

ARCHIVE_NAME="s3a-${TARGET}.${ARCHIVE_EXT}"
CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"

# Determine latest release tag
LATEST_TAG=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || echo "")
if [ -z "${LATEST_TAG}" ]; then
  LATEST_TAG="v0.1.0"
fi

BASE_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}"
DOWNLOAD_URL="${BASE_URL}/${ARCHIVE_NAME}"
CHECKSUM_URL="${BASE_URL}/${CHECKSUM_NAME}"

TMP_DIR=$(mktemp -d)
trap 'rm -rf "${TMP_DIR}"' EXIT

echo "Downloading S3A (${LATEST_TAG} for ${TARGET})..."
if ! curl -fsSL "${DOWNLOAD_URL}" -o "${TMP_DIR}/${ARCHIVE_NAME}"; then
  echo "Release asset not found at ${DOWNLOAD_URL}."
  echo "Falling back to 'cargo install s3a-cli'..."
  cargo install s3a-cli
  exit 0
fi

# Download and verify checksum if available
if curl -fsSL "${CHECKSUM_URL}" -o "${TMP_DIR}/${CHECKSUM_NAME}" 2>/dev/null; then
  echo "Verifying SHA-256 checksum..."
  cd "${TMP_DIR}"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "${CHECKSUM_NAME}" || { echo "Checksum verification failed!"; exit 1; }
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "${CHECKSUM_NAME}" || { echo "Checksum verification failed!"; exit 1; }
  fi
  cd - >/dev/null
fi

mkdir -p "${INSTALL_DIR}"
echo "Extracting binary to ${INSTALL_DIR}..."
tar -xzf "${TMP_DIR}/${ARCHIVE_NAME}" -C "${INSTALL_DIR}" s3a s3a-cli 2>/dev/null || tar -xzf "${TMP_DIR}/${ARCHIVE_NAME}" -C "${INSTALL_DIR}"

chmod +x "${INSTALL_DIR}/s3a" "${INSTALL_DIR}/s3a-cli" 2>/dev/null || true

# Check PATH
if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
  echo ""
  echo "Note: ${INSTALL_DIR} is not in your PATH."
  echo "Add it by adding the following to ~/.bashrc or ~/.zshrc:"
  echo '  export PATH="$HOME/.local/bin:$PATH"'
fi

echo ""
echo "================================================="
echo "S3A installed successfully!"
"${INSTALL_DIR}/s3a" --version || true
echo ""
echo "Next steps:"
echo "  1. Initialize your first vault:"
echo "     s3a init"
echo "  2. Run the interactive tour:"
echo "     s3a tour"
echo "  3. See available commands:"
echo "     s3a help"
echo "================================================="
