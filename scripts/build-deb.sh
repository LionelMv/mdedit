#!/usr/bin/env bash
# Build the .deb inside an Ubuntu 24.04 container, so it installs on Ubuntu 24.04 and newer
# (and other distros with glibc >= 2.39). A .deb built on the host would require the host's
# newer glibc.
#
# Needs podman (recommended: rootless, files stay yours) or docker.
# Output: target/ubuntu-24.04/debian/mdedit_<version>_amd64.deb
# The Rust toolchain and crates are cached in the volume "mdedit-deb-cargo"; remove it with
# `podman volume rm mdedit-deb-cargo` to start fresh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE="docker.io/library/ubuntu:24.04"
ENGINE="${ENGINE:-$(command -v podman || command -v docker || true)}"
if [ -z "${ENGINE}" ]; then
    echo "error: install podman (sudo apt install podman) or docker first" >&2
    exit 1
fi

"${ENGINE}" run --rm \
    -v "${ROOT}:/src:Z" -w /src \
    -v mdedit-deb-cargo:/root/.cargo \
    -e CARGO_TARGET_DIR=/src/target/ubuntu-24.04 \
    -e HOST_OWNER="$( [ "$(basename "${ENGINE}")" = docker ] && echo "$(id -u):$(id -g)" )" \
    "${IMAGE}" bash -euo pipefail -c '
        export DEBIAN_FRONTEND=noninteractive PATH=/root/.cargo/bin:$PATH
        apt-get update -qq
        apt-get install -y -qq --no-install-recommends ca-certificates curl build-essential dpkg-dev pkg-config >/dev/null
        if ! command -v cargo >/dev/null; then
            curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
        fi
        rustup update stable --no-self-update >/dev/null 2>&1 || true
        cargo deb --version >/dev/null 2>&1 || cargo install cargo-deb --locked
        cargo deb -p mdedit --locked
        # Docker runs as real root: hand the output back to the host user. (Under rootless
        # podman the container root already is the host user, so nothing to do.)
        if [ -n "${HOST_OWNER}" ]; then chown -R "${HOST_OWNER}" /src/target/ubuntu-24.04; fi
    '

echo "==> Built:"
ls -1 "${ROOT}"/target/ubuntu-24.04/debian/*.deb
