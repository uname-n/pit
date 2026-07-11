#!/bin/sh
set -e

REPO="uname-n/pit"
INSTALL_DIR="${PIT_INSTALL_DIR:-$HOME/.local/bin}"

detect_platform() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os="darwin" ;;
    Linux)  os="linux" ;;
    *) echo "error: unsupported OS: $os" >&2; exit 1 ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="x64" ;;
    arm64|aarch64) arch="arm64" ;;
    *) echo "error: unsupported architecture: $arch" >&2; exit 1 ;;
  esac

  echo "${os}-${arch}"
}

get_latest_tag() {
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | cut -d '"' -f 4
}

main() {
  platform="$(detect_platform)"
  asset="pit-${platform}"

  if [ -n "$1" ]; then
    tag="$1"
  else
    tag="$(get_latest_tag)"
  fi

  if [ -z "$tag" ]; then
    echo "error: could not determine latest release" >&2
    exit 1
  fi

  url="https://github.com/${REPO}/releases/download/${tag}/${asset}.tar.gz"

  echo "installing pit ${tag} (${platform})..."

  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  curl -fsSL "$url" | tar xz -C "$tmpdir"

  mkdir -p "$INSTALL_DIR"
  mv "$tmpdir/${asset}" "$INSTALL_DIR/pit"
  chmod +x "$INSTALL_DIR/pit"

  echo "installed pit to ${INSTALL_DIR}/pit"

  if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo ""
    echo "add to your PATH:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  fi
}

main "$@"
