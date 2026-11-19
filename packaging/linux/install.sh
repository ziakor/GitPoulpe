#!/bin/sh
# Installs GitPoulpe from the unpacked tarball. Usage: ./install.sh [--prefix DIR] [--force]
# Default prefix: $HOME/.local. Nothing outside the prefix is written.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix=${HOME:-}/.local
force=0
while [ $# -gt 0 ]; do
  case $1 in
    --prefix) [ $# -ge 2 ] || { echo "--prefix needs a directory" >&2; exit 2; }; prefix=$2; shift 2 ;;
    --force) force=1; shift ;;
    -h|--help) sed -n '2,3p' "$0" | sed 's/^# //'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
case $prefix in /*) ;; *) echo "the prefix must be an absolute path: $prefix" >&2; exit 2 ;; esac

[ -x "$here/bin/gitpoulpe" ] || { echo "bin/gitpoulpe is missing: run this script from the unpacked package" >&2; exit 1; }

ldd_cmd=${GITPOULPE_LDD:-ldd}
if command -v "$ldd_cmd" >/dev/null 2>&1; then
  missing=$("$ldd_cmd" "$here/bin/gitpoulpe" 2>/dev/null | sed -n 's/^[[:space:]]*\(.*\) => not found.*/\1/p')
  if [ -n "$missing" ] && [ "$force" -ne 1 ]; then
    echo "Missing shared libraries, install them with your package manager first (or use --force):" >&2
    echo "$missing" | sed 's/^/  /' >&2
    exit 1
  fi
fi
command -v git >/dev/null 2>&1 || echo "Warning: git is not installed; GitPoulpe needs it to open repositories." >&2

manifest="$prefix/share/gitpoulpe/install-manifest.txt"
files="bin/gitpoulpe share/applications/gitpoulpe.desktop share/licenses/gitpoulpe/LICENSE share/licenses/gitpoulpe/LICENSE-IBM-Plex-Sans.txt share/licenses/gitpoulpe/LICENSE-IBM-Plex-Mono.txt share/doc/gitpoulpe/README.md share/doc/gitpoulpe/PROVENANCE.txt"

install -Dm755 "$here/bin/gitpoulpe" "$prefix/bin/gitpoulpe"
mkdir -p "$prefix/share/applications"
sed "s|@PREFIX@|$prefix|g" "$here/share/applications/gitpoulpe.desktop.in" > "$prefix/share/applications/gitpoulpe.desktop"
chmod 644 "$prefix/share/applications/gitpoulpe.desktop"
for f in share/licenses/gitpoulpe/LICENSE share/licenses/gitpoulpe/LICENSE-IBM-Plex-Sans.txt share/licenses/gitpoulpe/LICENSE-IBM-Plex-Mono.txt share/doc/gitpoulpe/README.md share/doc/gitpoulpe/PROVENANCE.txt; do
  install -Dm644 "$here/$f" "$prefix/$f"
done
mkdir -p "$(dirname "$manifest")"
for f in $files; do echo "$prefix/$f"; done > "$manifest"
echo "$manifest" >> "$manifest"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
echo "Installed to $prefix. Run: $prefix/bin/gitpoulpe --version"
case ":${PATH:-}:" in *":$prefix/bin:"*) ;; *) echo "Note: $prefix/bin is not in your PATH." ;; esac
