#!/usr/bin/env bash
# Remove a user install made by scripts/install.sh.
# Usage: scripts/uninstall.sh [--prefix DIR]   (default prefix: $HOME/.local)
# User config (~/.config/mdedit) and state (~/.local/state/mdedit) are left alone.
set -euo pipefail

PREFIX="${HOME}/.local"
APP_ID="io.github.mdedit.mdedit"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        -h|--help) sed -n '2,4p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

SHARE="${PREFIX}/share"
ICONS="${SHARE}/icons/hicolor"

files=(
    "${PREFIX}/bin/mdedit"
    "${SHARE}/applications/${APP_ID}.desktop"
    "${SHARE}/metainfo/${APP_ID}.metainfo.xml"
    "${ICONS}/scalable/apps/${APP_ID}.svg"
)
for size in 48 128 256 512; do
    files+=("${ICONS}/${size}x${size}/apps/${APP_ID}.png")
done

for f in "${files[@]}"; do
    if [[ -e "${f}" ]]; then
        rm -f -- "${f}"
        echo "removed ${f}"
    fi
done

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "${SHARE}/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -d "${ICONS}" ]]; then
    gtk-update-icon-cache -q -t -f "${ICONS}" || true
fi
echo "==> mdedit uninstalled from ${PREFIX}"
