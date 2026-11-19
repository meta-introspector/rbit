#!/usr/bin/env bash
# Installs a prebuilt cargo-deny into the directory given as $1, pinned by version and
# SHA-256 so CI never runs an unchecked download. The musl builds support Linux x86_64
# and aarch64. To update, use the new release version and its published `.sha256` files.
set -euo pipefail
version="0.20.2"
dest="${1:?usage: install-cargo-deny.sh DIR}"
arch="$(uname -m)"
case "$arch" in
    x86_64) sha256="9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f" ;;
    aarch64) sha256="995c82be0defc7a025cae49a2aa2644ce8245c9a3318fc4103907c6a285e8c7d" ;;
    *) echo "Unsupported Linux architecture for cargo-deny: $arch" >&2; exit 1 ;;
esac
name="cargo-deny-${version}-${arch}-unknown-linux-musl"
mkdir -p "$dest"
curl -fsSL --retry 3 -o "$dest/$name.tar.gz" "https://github.com/EmbarkStudios/cargo-deny/releases/download/${version}/${name}.tar.gz"
echo "${sha256}  $dest/$name.tar.gz" | sha256sum --check --quiet
tar -xzf "$dest/$name.tar.gz" -C "$dest" --strip-components=1 "$name/cargo-deny"
rm "$dest/$name.tar.gz"
"$dest/cargo-deny" --version
