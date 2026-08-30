#!/bin/sh
# Install the latest tinyanalyzer release on Linux or macOS.
set -eu

repository="tinyhumansai/tinyanalyzer"
binary="tinyanalyzer"

command -v curl >/dev/null 2>&1 || {
    echo "tinyanalyzer installer: curl is required" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) platform="linux" ;;
    Darwin) platform="macos" ;;
    *)
        echo "tinyanalyzer installer: unsupported operating system: $(uname -s)" >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) architecture="x86_64" ;;
    arm64 | aarch64) architecture="aarch64" ;;
    *)
        echo "tinyanalyzer installer: unsupported architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

version="${TINYANALYZER_VERSION:-}"
if [ -z "$version" ]; then
    latest_url="$(curl --proto '=https' --tlsv1.2 -fsSL -o /dev/null -w '%{url_effective}' \
        "https://github.com/${repository}/releases/latest")"
    version="${latest_url##*/}"
fi

case "$version" in
    v*) ;;
    *) version="v${version}" ;;
esac

asset="${binary}-${version}-${platform}-${architecture}.tar.gz"
release_url="https://github.com/${repository}/releases/download/${version}"
temporary="$(mktemp -d "${TMPDIR:-/tmp}/tinyanalyzer-install.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

echo "Downloading ${asset}"
curl --proto '=https' --tlsv1.2 -fsSL "${release_url}/${asset}" -o "${temporary}/${asset}"
curl --proto '=https' --tlsv1.2 -fsSL "${release_url}/SHA256SUMS" \
    -o "${temporary}/SHA256SUMS"

expected="$(awk -v asset="$asset" '$2 == asset || $2 == "./" asset { print $1; exit }' \
    "${temporary}/SHA256SUMS")"
[ -n "$expected" ] || {
    echo "tinyanalyzer installer: ${asset} is absent from SHA256SUMS" >&2
    exit 1
}

if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${temporary}/${asset}" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "${temporary}/${asset}" | awk '{ print $1 }')"
else
    echo "tinyanalyzer installer: sha256sum or shasum is required" >&2
    exit 1
fi

[ "$actual" = "$expected" ] || {
    echo "tinyanalyzer installer: checksum verification failed" >&2
    exit 1
}

tar -xzf "${temporary}/${asset}" -C "$temporary"
source_binary="${temporary}/${asset%.tar.gz}/${binary}"
[ -f "$source_binary" ] || {
    echo "tinyanalyzer installer: archive does not contain ${binary}" >&2
    exit 1
}

install_directory="${TINYANALYZER_INSTALL_DIR:-${HOME}/.local/bin}"
mkdir -p "$install_directory"
install -m 0755 "$source_binary" "${install_directory}/${binary}"

echo "Installed tinyanalyzer ${version} to ${install_directory}/${binary}"
case ":${PATH}:" in
    *":${install_directory}:"*) ;;
    *) echo "Add ${install_directory} to PATH to run tinyanalyzer." ;;
esac
