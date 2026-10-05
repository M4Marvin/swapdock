#!/usr/bin/env bash
# install-deploy.sh — one-time bootstrap for the deploy tool.
#
#   sudo bash ~/deploy-dist/install-deploy.sh ~/deploy-dist
#
# What it does: installs the binary, creates /srv/deploy, installs the two
# hand-written nginx files, adds passwordless sudo for the deploy binary, and
# proves the existing nginx config still passes.
#
# What it does NOT do: reload nginx, touch the tunnel, move any traffic, or
# restart anything. The running state is byte-identical afterwards.
#
# Idempotent: safe to re-run. Existing files it would overwrite are kept as
# .bak first, and it aborts on the first failure.
set -euo pipefail

DIST="${1:-/home/marv/deploy-dist}"
test -d "$DIST" || { echo "dist dir missing: $DIST" >&2; exit 1; }
test "$(id -u)" = "0" || { echo "run as root: sudo bash $0 $DIST" >&2; exit 1; }

say() { printf '== %s ==\n' "$*"; }

say "preconditions"
command -v nginx >/dev/null || { echo "nginx not installed" >&2; exit 1; }
command -v docker >/dev/null || { echo "docker not installed" >&2; exit 1; }
test -x "$DIST/deploy" || { echo "$DIST/deploy missing or not executable" >&2; exit 1; }
"$DIST/deploy" --version

say "back up /etc/nginx"
BACKUP="/root/nginx-pre-deploy-$(date +%F-%H%M).tgz"
tar -czf "$BACKUP" -C / etc/nginx
echo "backup: $BACKUP"

say "install the binary"
install -m 0755 "$DIST/deploy" /usr/local/bin/deploy

say "create /srv/deploy and /var/log/deploy"
mkdir -p /srv/deploy/apps /srv/deploy/green /var/log/deploy
# -n: never overwrite. Re-runs keep operator edits; new apps still get copied.
cp -n "$DIST"/apps/*.toml /srv/deploy/apps/
chown -R marv:marv /srv/deploy/apps
chown root:root /srv/deploy/green /var/log/deploy
chmod 0755 /srv/deploy /srv/deploy/apps /srv/deploy/green /var/log/deploy
echo "registry files: $(ls /srv/deploy/apps/*.toml | wc -l)"

say "install hand-written nginx files"
for f in snippets/proxy-common.conf conf.d/deploy-http.conf; do
    src="$DIST/nginx/$(basename "$f")"
    dst="/etc/nginx/$f"
    if test -e "$dst"; then
        cp -n "$dst" "$dst.bak" && echo "kept existing $dst as $dst.bak"
    fi
    install -m 0644 "$src" "$dst"
    echo "wrote $dst"
done

say "prove the config still passes"
nginx -t

say "passwordless sudo for the deploy binary"
printf '%s\n' "marv ALL=(root) NOPASSWD: /usr/local/bin/deploy" > /etc/sudoers.d/deploy
chmod 0440 /etc/sudoers.d/deploy
visudo -c -f /etc/sudoers.d/deploy

say "log rotation for the run log"
install -m 0644 "$DIST/logrotate-deploy" /etc/logrotate.d/deploy

say "done — running state untouched (no reload, no restart)"
echo
echo "Next, as marv (no sudo needed for these):"
echo "  deploy --registry /srv/deploy/apps --tunnel-config ~/.cloudflared/config.yaml validate"
echo "  deploy --registry /srv/deploy/apps render | head -30"
echo
echo "Then, when ready, the first apply (reloads nginx with an equivalent config):"
echo "  sudo -n /usr/local/bin/deploy --registry /srv/deploy/apps \\"
echo "      --trace /var/log/deploy/deploy.jsonl apply"
