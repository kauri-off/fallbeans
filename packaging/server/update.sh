#!/bin/sh
# Updates fallbeans-server on Debian/Ubuntu to the latest GitHub release and restarts it. Usage: update.sh [-f]
set -eu

REPO=kauri-off/fallbeans
PKG=fallbeans-server
SERVICE=fallbeans

force=0
case "${1:-}" in
  -f | --force) force=1 ;;
  "") ;;
  *)
    echo "usage: $0 [-f|--force]" >&2
    exit 2
    ;;
esac

say() { printf '==> %s\n' "$*"; }
die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[ "$(id -u)" -eq 0 ] || die "run as root: sudo $0"
command -v curl >/dev/null || die "curl is missing: apt install curl"
[ "$(dpkg --print-architecture)" = amd64 ] || die "only amd64 packages are published"

say "Checking the latest release of $REPO"
json=$(curl -fsSL -H "Accept: application/vnd.github+json" "https://api.github.com/repos/$REPO/releases/latest") ||
  die "GitHub API request failed"
tag=$(printf '%s' "$json" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$tag" ] || die "no tag_name in the GitHub response"
deb_url=$(printf '%s' "$json" | sed -n 's/.*"browser_download_url": *"\([^"]*_amd64\.deb\)".*/\1/p' | head -n1)
sums_url=$(printf '%s' "$json" | sed -n 's/.*"browser_download_url": *"\([^"]*\/SHA256SUMS\)".*/\1/p' | head -n1)
[ -n "$deb_url" ] || die "release $tag has no .deb"

# Tag v0.1.0-alpha.2 is Debian version 0.1.0~alpha.2.
new=$(printf '%s' "${tag#v}" | sed 's/-/~/')
old=$(dpkg-query -W -f '${Version}' "$PKG" 2>/dev/null || true)
say "Installed: ${old:-none}, latest: $new ($tag)"

if [ -n "$old" ] && [ "$force" -eq 0 ] && dpkg --compare-versions "$old" ge "$new"; then
  say "Already up to date, nothing to do (-f to reinstall)"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
deb="$tmp/${deb_url##*/}"

say "Downloading ${deb_url##*/}"
curl -fL --retry 3 -o "$deb" "$deb_url" || die "download failed"

if [ -n "$sums_url" ]; then
  say "Verifying SHA256"
  curl -fsSL --retry 3 -o "$tmp/SHA256SUMS" "$sums_url" || die "SHA256SUMS download failed"
  (cd "$tmp" && grep " ${deb##*/}\$" SHA256SUMS | sha256sum -c -) || die "checksum mismatch"
else
  say "No SHA256SUMS in the release, skipping verification"
fi

say "Installing"
chmod 644 "$deb"
DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades "$deb"

say "Restarting $SERVICE"
systemctl daemon-reload
systemctl restart "$SERVICE"
sleep 2

installed=$(dpkg-query -W -f '${Version}' "$PKG")
state=$(systemctl is-active "$SERVICE" || true)

echo
say "Done"
echo "  package:  $PKG ${old:-none} -> $installed"
echo "  release:  https://github.com/$REPO/releases/tag/$tag"
echo "  service:  $SERVICE is $state"
echo "  logs:     journalctl -u $SERVICE -n 50"
echo
journalctl -u "$SERVICE" -n 10 --no-pager || true

[ "$state" = active ] || die "$SERVICE is not running after the update"
