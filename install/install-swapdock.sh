#!/usr/bin/env bash
# install-swapdock.sh — one-time bootstrap for the swapdock tool.
#
#   sudo bash ~/swapdock-dist/install-swapdock.sh ~/swapdock-dist
#
# What it does: installs the binary, creates /srv/swapdock, installs the three
# hand-written nginx files, adds passwordless sudo for the swapdock binary, and
# proves the existing nginx config still passes.
#
# What it does NOT do: reload nginx, touch the tunnel, move any traffic, or
# restart anything. The running state is byte-identical afterwards.
#
# Idempotent: safe to re-run. Existing files it would overwrite are kept as
# .bak first, and it aborts on the first failure.
set -euo pipefail

# Under sudo, $HOME is /root, so default to the invoking user's home instead.
DIST="${1:-${SUDO_USER:+/home/$SUDO_USER/swapdock-dist}}"
test -n "$DIST" || { echo "usage: sudo bash $0 /path/to/swapdock-dist" >&2; exit 1; }
test -d "$DIST" || { echo "dist dir missing: $DIST" >&2; exit 1; }
test "$(id -u)" = "0" || { echo "run as root: sudo bash $0 $DIST" >&2; exit 1; }

# The user who invoked sudo owns the registry. Refuse a bare root login:
# there would be no one to hand the files to.
INSTALL_USER="${SUDO_USER:-}"
test -n "$INSTALL_USER" || { echo "run via sudo, not as root directly" >&2; exit 1; }
id "$INSTALL_USER" >/dev/null 2>&1 || { echo "unknown user: $INSTALL_USER" >&2; exit 1; }
# Group that owns the run log, so unprivileged commands can append to it. The
# group is not always the username: ask, do not assume.
LOG_GROUP="$(id -gn "$INSTALL_USER")"

say() { printf '== %s ==\n' "$*"; }

say "preconditions"
command -v nginx >/dev/null || { echo "nginx not installed" >&2; exit 1; }
command -v docker >/dev/null || { echo "docker not installed" >&2; exit 1; }
test -x "$DIST/swapdock" || { echo "$DIST/swapdock missing or not executable" >&2; exit 1; }
"$DIST/swapdock" --version

say "back up /etc/nginx"
BACKUP="/root/nginx-pre-swapdock-$(date +%F-%H%M).tgz"
tar -czf "$BACKUP" -C / etc/nginx
echo "backup: $BACKUP"

say "install the binary"
install -m 0755 "$DIST/swapdock" /usr/local/bin/swapdock

say "create /srv/swapdock and /var/log/swapdock"
mkdir -p /srv/swapdock/apps /srv/swapdock/green
# -n: never overwrite. Re-runs keep operator edits; new apps still get copied.
cp -n "$DIST"/apps/*.toml /srv/swapdock/apps/
chown -R "$INSTALL_USER:$INSTALL_USER" /srv/swapdock/apps
chown root:root /srv/swapdock/green
chmod 0755 /srv/swapdock /srv/swapdock/apps /srv/swapdock/green
# The run log is appended by unprivileged commands too, so it and its directory
# belong to the deploy user's group. touch, never truncate: this re-runs.
install -d -m 2770 -o root -g "$LOG_GROUP" /var/log/swapdock
touch /var/log/swapdock/swapdock.jsonl
chown root:"$LOG_GROUP" /var/log/swapdock/swapdock.jsonl
chmod 0664 /var/log/swapdock/swapdock.jsonl
echo "registry files: $(ls /srv/swapdock/apps/*.toml | wc -l)"

say "install hand-written nginx files"
for f in snippets/proxy-common.conf snippets/spa-fallback.conf conf.d/00-swapdock-http.conf; do
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

say "passwordless sudo for the swapdock binary"
printf '%s\n' "$INSTALL_USER ALL=(root) NOPASSWD: /usr/local/bin/swapdock" > /etc/sudoers.d/swapdock
chmod 0440 /etc/sudoers.d/swapdock
visudo -c -f /etc/sudoers.d/swapdock

say "log rotation for the run log"
# Substituted, not copied: a rotation that recreates the log root-only breaks
# every unprivileged command until the next install.
sed "s/@LOG_GROUP@/$LOG_GROUP/" "$DIST/logrotate-swapdock" > /etc/logrotate.d/swapdock
chmod 0644 /etc/logrotate.d/swapdock

say "let $INSTALL_USER read the nginx access log"
# verify reads the front-door access log, which is www-data:adm 0640.
if id -nG "$INSTALL_USER" | grep -qw adm; then
    echo "$INSTALL_USER is already in adm"
else
    usermod -aG adm "$INSTALL_USER"
    echo "added $INSTALL_USER to adm; log out and back in for verify to use it"
fi

say "done — running state untouched (no reload, no restart)"
echo
echo "Next, as $INSTALL_USER (no sudo needed for these):"
echo "  swapdock --registry /srv/swapdock/apps validate"
echo "  swapdock --registry /srv/swapdock/apps render | head -30"
echo
echo "Then, when ready, the first apply (reloads nginx with an equivalent config):"
echo "  sudo -n /usr/local/bin/swapdock --registry /srv/swapdock/apps \\"
echo "      --trace /var/log/swapdock/swapdock.jsonl apply"
