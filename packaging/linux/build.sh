#!/bin/sh
# Builds the Linux release tarball: dist/gitpoulpe-<version>-linux-<arch>.tar.gz (+ .sha256).
# Usage: packaging/linux/build.sh [output-dir]   Optional: TOOLCHAIN=1.100.0
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:-$root/dist}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
arch=$(uname -m)
commit=$(git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
if [ "$commit" != unknown ] && [ -n "$(git -C "$root" status --porcelain 2>/dev/null)" ]; then
  commit="$commit-dirty"
fi
name="gitpoulpe-$version-linux-$arch"
cargo_cmd=cargo
[ -n "${TOOLCHAIN:-}" ] && cargo_cmd="cargo +$TOOLCHAIN"

cd "$root"
GITPOULPE_BUILD_COMMIT=$commit $cargo_cmd build --release --locked -p gitpoulpe-desktop

stage="$out/$name"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/licenses/gitpoulpe" "$stage/share/doc/gitpoulpe"
install -m755 target/release/gitpoulpe "$stage/bin/gitpoulpe"
install -m644 packaging/linux/gitpoulpe.desktop.in "$stage/share/applications/gitpoulpe.desktop.in"
install -m644 LICENSE "$stage/share/licenses/gitpoulpe/LICENSE"
install -m644 crates/gitpoulpe-desktop/assets/fonts/LICENSE-IBM-Plex-Sans.txt crates/gitpoulpe-desktop/assets/fonts/LICENSE-IBM-Plex-Mono.txt "$stage/share/licenses/gitpoulpe/"
install -m644 README.md "$stage/share/doc/gitpoulpe/README.md"
install -m755 packaging/linux/install.sh packaging/linux/uninstall.sh "$stage/"

{
  echo "GitPoulpe $version"
  echo "Source commit: $commit"
  echo "Target: linux-$arch"
  echo "Compiler: $($cargo_cmd rustc -- -V 2>/dev/null | head -n 1 || true)"
  echo "License: GPL-3.0-only (share/licenses/gitpoulpe/LICENSE)"
  echo
  echo "SHA-256 of the build inputs and outputs:"
  (cd "$root" && sha256sum Cargo.lock) | sed 's/^/  /'
  (cd "$stage" && sha256sum bin/gitpoulpe) | sed 's/^/  /'
  echo
  echo "Embedded fonts (IBM Plex, https://github.com/IBM/plex, SIL Open Font License 1.1):"
  (cd "$root/crates/gitpoulpe-desktop/assets/fonts" && sha256sum ./*.ttf) | sed 's/^/  /'
  echo
  echo "Run 'gitpoulpe --licenses' for the same notices from the installed binary."
} > "$stage/share/doc/gitpoulpe/PROVENANCE.txt"

tarball="$out/$name.tar.gz"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -C "$out" -cf - "$name" | gzip -n > "$tarball"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "$tarball"
