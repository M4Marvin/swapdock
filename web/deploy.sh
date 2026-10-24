#!/usr/bin/env bash
# deploy.sh — ship the built SPA to the host as a release, then flip `current`.
#
#   ./deploy.sh [release-id]
#
# Builds `dist/`, rsyncs it to <host>:/srv/www/swapdock-ui/releases/<id>, and
# atomically repoints the `current` symlink nginx serves. nginx needs no change:
# its `root` is the stable symlink, so the flip is the whole deploy.
#
# Default release id: the short commit, plus `-dirty<YYYYmmddHHMM>` when the
# tree has uncommitted changes — a release must never masquerade as a commit
# that does not contain it. Both the SSH destination and the verification URL
# are required: this is a local helper, not an installer, and it has no
# opinion about your host.
#
#   SWAPDOCK_WEB_HOST=deploy@example.com \
#   SWAPDOCK_WEB_URL=https://console.example.com/ ./deploy.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
host="${SWAPDOCK_WEB_HOST:?set SWAPDOCK_WEB_HOST to the ssh destination}"
root="/srv/www/swapdock-ui"
url="${SWAPDOCK_WEB_URL:?set SWAPDOCK_WEB_URL to the console URL to verify}"

rev="$(git -C "$here" rev-parse --short HEAD 2>/dev/null || date +%Y%m%d%H%M)"
if test -n "$(git -C "$here" status --porcelain -- . 2>/dev/null)"; then
    rev="$rev-dirty$(date +%Y%m%d%H%M)"
fi
release="${1:-$rev}"

case "$release" in
    *[!a-z0-9.-]*)
        echo "release id must be [a-z0-9.-]: $release" >&2
        exit 1
        ;;
esac

printf '== build (%s)\n' "$release"
(cd "$here" && pnpm build)

printf '== ship to %s:%s/releases/%s\n' "$host" "$root" "$release"
# Normalize modes: a build under umask 007 ships 770/660 and nginx answers 403.
rsync -az --delete --chmod=D755,F644 "$here/dist/" "$host:$root/releases/$release/"

printf '== flip current\n'
ssh "$host" "ln -sfn 'releases/$release' '$root/current.tmp' && mv -T '$root/current.tmp' '$root/current'"

printf '== verify %s\n' "$url"
curl -fsS -o /dev/null -w 'serving %{http_code}\n' "$url"
