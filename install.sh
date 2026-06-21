#!/usr/bin/env bash
set -e

REPO="MonilMehta/scrub"
BIN_NAME="scrub"

echo "Installing $BIN_NAME..."

# Detect OS
OS="$(uname -s)"
case "${OS}" in
    Linux*)     OS_NAME="unknown-linux-gnu"; EXT="tar.gz";;
    Darwin*)    OS_NAME="apple-darwin"; EXT="tar.gz";;
    CYGWIN*|MINGW*|MSYS*) OS_NAME="pc-windows-msvc"; EXT="zip";;
    *)          echo "Unsupported OS: ${OS}"; exit 1;;
esac

# Detect Architecture
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64*|amd64*) ARCH_NAME="x86_64";;
    aarch64*|arm64*) ARCH_NAME="aarch64";;
    *)               echo "Unsupported architecture: ${ARCH}"; exit 1;;
esac

TARGET="${ARCH_NAME}-${OS_NAME}"
if [ "${OS_NAME}" = "pc-windows-msvc" ]; then
    FILE_NAME="${BIN_NAME}-${TARGET}.${EXT}"
else
    FILE_NAME="${BIN_NAME}-${TARGET}.${EXT}"
fi

# Get latest release tag
LATEST_RELEASE=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$LATEST_RELEASE" ]; then
    echo "Error: Could not determine latest release version."
    exit 1
fi

DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_RELEASE}/${FILE_NAME}"

echo "Downloading $DOWNLOAD_URL"

TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"

curl -sL "$DOWNLOAD_URL" -o "$FILE_NAME"

if [ "${EXT}" = "zip" ]; then
    unzip -q "$FILE_NAME"
else
    tar -xzf "$FILE_NAME"
fi

INSTALL_DIR="/usr/local/bin"
if [ ! -w "$INSTALL_DIR" ]; then
    echo "Requires sudo to install to $INSTALL_DIR"
    sudo mv "$BIN_NAME" "$INSTALL_DIR/"
else
    mv "$BIN_NAME" "$INSTALL_DIR/"
fi

chmod +x "$INSTALL_DIR/$BIN_NAME"

rm -rf "$TMP_DIR"

echo "Successfully installed $BIN_NAME to $INSTALL_DIR"
echo "Run 'scrub dashboard' to get started."
