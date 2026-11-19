#!/bin/sh
# End-to-end check of the Linux package: build, install into a temporary prefix, inspect, uninstall.
# Usage: packaging/linux/test.sh   (needs a release build, git, desktop-file-validate)
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

tarball=$("$root/packaging/linux/build.sh" "$work/dist" | tail -n 1)
[ -f "$tarball" ] || fail "no tarball produced"
(cd "$(dirname "$tarball")" && sha256sum -c "$(basename "$tarball").sha256" >/dev/null) || fail "checksum does not match"
ok "tarball built and checksum verified"

mkdir "$work/unpack" && tar -xzf "$tarball" -C "$work/unpack"
pkg=$(echo "$work"/unpack/gitpoulpe-*)

home="$work/home with space"
mkdir -p "$home/.config/gitpoulpe" "$home/code/repo"
echo '{"format":1}' > "$home/.config/gitpoulpe/prefs.json"
git -C "$home/code/repo" init -q
echo keep > "$home/code/repo/file.txt"
prefix="$work/prefix with space"

HOME=$home "$pkg/install.sh" --prefix "$prefix" >/dev/null || fail "install failed"
[ -x "$prefix/bin/gitpoulpe" ] || fail "binary missing"
version=$("$prefix/bin/gitpoulpe" --version)
echo "$version" | grep -q '^gitpoulpe [0-9]' || fail "bad --version: $version"
echo "$version" | grep -q 'GPL-3.0-only' || fail "licence missing from --version"
"$prefix/bin/gitpoulpe" --licenses | grep -q 'SIL Open Font License' || fail "font licence missing"
grep -q "IBMPlexSans-Regular.ttf" "$prefix/share/doc/gitpoulpe/PROVENANCE.txt" || fail "font provenance missing"
grep -q "^  [0-9a-f]\{64\}  bin/gitpoulpe$" "$prefix/share/doc/gitpoulpe/PROVENANCE.txt" || fail "binary hash missing"
(cd "$prefix" && sed -n 's/^  \([0-9a-f]\{64\}  bin\/gitpoulpe\)$/\1/p' share/doc/gitpoulpe/PROVENANCE.txt | sha256sum -c >/dev/null) || fail "installed binary does not match its recorded hash"
grep -q "^Exec=$prefix/bin/gitpoulpe %u$" "$prefix/share/applications/gitpoulpe.desktop" || fail "desktop Exec not rewritten"
desktop-file-validate "$prefix/share/applications/gitpoulpe.desktop" || fail "desktop file invalid"
[ -f "$prefix/share/licenses/gitpoulpe/LICENSE" ] || fail "GPL text missing"
ok "install exposes version, licence, provenance and a valid launcher"

HOME=$home "$pkg/install.sh" --prefix "$prefix" >/dev/null || fail "reinstall failed"
ok "reinstall over an existing installation"

cat > "$work/fake-ldd" <<'LDD'
#!/bin/sh
echo "	libmissing.so.1 => not found"
LDD
chmod +x "$work/fake-ldd"
if GITPOULPE_LDD="$work/fake-ldd" HOME=$home "$pkg/install.sh" --prefix "$work/never" 2>"$work/err"; then
  fail "install should refuse when a library is missing"
fi
grep -q libmissing "$work/err" || fail "missing library not named"
[ ! -e "$work/never" ] || fail "refused install still wrote files"
GITPOULPE_LDD="$work/fake-ldd" HOME=$home "$pkg/install.sh" --prefix "$work/forced" --force >/dev/null || fail "--force should install"
ok "missing libraries are reported before anything is written"

HOME=$home "$pkg/uninstall.sh" --prefix "$work/forced" >/dev/null || fail "uninstall of forced prefix"
out=$(HOME=$home "$pkg/uninstall.sh" --prefix "$prefix") || fail "uninstall failed"
echo "$out" | grep -q "Preferences kept" || fail "no notice about kept preferences"
[ ! -e "$prefix/bin/gitpoulpe" ] && [ ! -e "$prefix/share/applications/gitpoulpe.desktop" ] || fail "files left behind"
[ ! -e "$prefix/share/gitpoulpe" ] && [ ! -e "$prefix/share/licenses/gitpoulpe" ] || fail "directories left behind"
[ -f "$home/.config/gitpoulpe/prefs.json" ] || fail "preferences were deleted without being asked"
[ "$(cat "$home/code/repo/file.txt")" = keep ] && [ -d "$home/code/repo/.git" ] || fail "repository touched"
ok "uninstall keeps preferences and repositories"

if HOME=$home "$pkg/uninstall.sh" --prefix "$prefix" 2>/dev/null; then fail "second uninstall should report nothing to remove"; fi
HOME=$home "$pkg/install.sh" --prefix "$prefix" >/dev/null
HOME=$home "$pkg/uninstall.sh" --prefix "$prefix" --purge-config >/dev/null || fail "purge failed"
[ ! -e "$home/.config/gitpoulpe" ] || fail "--purge-config did not remove preferences"
[ -d "$home/code/repo/.git" ] || fail "purge touched the repository"
ok "--purge-config removes only the preferences"

echo "All package checks passed."
