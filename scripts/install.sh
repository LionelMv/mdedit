#!/usr/bin/env bash
# Build mdedit in release mode and install it for the current user under ~/.local.
# Usage: scripts/install.sh [--no-build] [--prefix DIR]   (default prefix: $HOME/.local)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${HOME}/.local"
BUILD=1
APP_ID="io.github.mdedit.mdedit"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-build) BUILD=0; shift ;;
        --prefix) PREFIX="$2"; shift 2 ;;
        -h|--help) sed -n '2,3p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

CARGO="${CARGO:-$(command -v cargo || echo "${HOME}/.cargo/bin/cargo")}"
TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT}/target}"
BIN="${TARGET_DIR}/release/mdedit"
PKG="${ROOT}/packaging/linux"

if [[ "${BUILD}" -eq 1 ]]; then
    echo "==> Building mdedit (release)"
    (cd "${ROOT}" && "${CARGO}" build --release -p mdedit)
fi
[[ -x "${BIN}" ]] || { echo "error: ${BIN} not found; build first" >&2; exit 1; }

SHARE="${PREFIX}/share"
ICONS="${SHARE}/icons/hicolor"

echo "==> Installing into ${PREFIX}"
install -Dm755 "${BIN}" "${PREFIX}/bin/mdedit"
install -Dm644 "${PKG}/${APP_ID}.desktop" "${SHARE}/applications/${APP_ID}.desktop"
install -Dm644 "${PKG}/${APP_ID}.metainfo.xml" "${SHARE}/metainfo/${APP_ID}.metainfo.xml"
install -Dm644 "${PKG}/icons/${APP_ID}.svg" "${ICONS}/scalable/apps/${APP_ID}.svg"
for size in 48 128 256 512; do
    install -Dm644 "${PKG}/icons/${APP_ID}-${size}.png" "${ICONS}/${size}x${size}/apps/${APP_ID}.png"
done

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "${SHARE}/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "${ICONS}" || true
fi

echo "==> Installed ${PREFIX}/bin/mdedit"
case ":${PATH}:" in
    *":${PREFIX}/bin:"*) ;;
    *) echo "note: ${PREFIX}/bin is not on your PATH" ;;
esac
