#!/bin/sh
# Removes what install.sh installed. Usage: ./uninstall.sh [--prefix DIR] [--purge-config]
# Repositories are never touched. Preferences are kept unless --purge-config is given.
set -eu

prefix=${HOME:-}/.local
purge=0
while [ $# -gt 0 ]; do
  case $1 in
    --prefix) [ $# -ge 2 ] || { echo "--prefix needs a directory" >&2; exit 2; }; prefix=$2; shift 2 ;;
    --purge-config) purge=1; shift ;;
    -h|--help) sed -n '2,3p' "$0" | sed 's/^# //'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
case $prefix in /*) ;; *) echo "the prefix must be an absolute path: $prefix" >&2; exit 2 ;; esac

manifest="$prefix/share/gitpoulpe/install-manifest.txt"
[ -f "$manifest" ] || { echo "No installation found under $prefix (missing $manifest)." >&2; exit 1; }

while IFS= read -r file; do
  case $file in "$prefix"/*) rm -f -- "$file" ;; *) echo "Skipping $file: outside the prefix" >&2 ;; esac
done < "$manifest"
for dir in share/licenses/gitpoulpe share/doc/gitpoulpe share/gitpoulpe; do
  rmdir "$prefix/$dir" 2>/dev/null || true
done

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true

config=${RUNE_PREFS_DIR:-${XDG_CONFIG_HOME:-${HOME:-}/.config}}/gitpoulpe
if [ "$purge" -eq 1 ]; then
  if [ -d "$config" ]; then rm -rf -- "$config"; echo "Removed preferences: $config"; fi
else
  [ -d "$config" ] && echo "Preferences kept in $config (use --purge-config to delete them)."
fi
echo "GitPoulpe was removed from $prefix. Your repositories were not touched."
