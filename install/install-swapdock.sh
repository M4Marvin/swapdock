#!/usr/bin/env bash
# install-swapdock.sh — install the swapdock binary and host integration files.
#
#   sudo bash install-swapdock.sh [--dry-run] [--with-service] [--non-root] [--with-backup-timer] [--with-monitor-timer] [DIST_DIR]
#
# What it does:
#   - installs the binary at /usr/local/bin/swapdock (previous one kept as .bak)
#   - creates /srv/swapdock/{apps,green,overrides,backups} and /var/log/swapdock
#   - copies apps/*.toml into the registry without overwriting existing files
#   - installs the three hand-written nginx files (previous ones kept as .bak)
#   - installs a passwordless sudo entry for the binary and log rotation
#   - with --with-service, installs the example systemd unit but does NOT start it
#   - with --non-root, installs the API as the deploy user plus the root apply broker
#   - with --with-backup-timer, installs the daily backup timer but does NOT enable it
#   - with --with-monitor-timer, installs the five-minute monitor timer but does NOT enable it
#
# What it does NOT do: reload nginx, start the API, move traffic, restart an app
# container, or edit the tunnel. The running state is unchanged when it exits.
#
# Idempotent: safe to re-run. Files it replaces are backed up once as .bak.
# --dry-run prints the plan, changes nothing, and does not need root.
set -euo pipefail

DIST=""
DRY_RUN=0
WITH_SERVICE=0
WITH_BACKUP_TIMER=0
WITH_MONITOR_TIMER=0
NON_ROOT=0

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

say() {
    printf '== %s ==\n' "$*"
}

usage() {
    cat <<'EOF'
usage: sudo bash install-swapdock.sh [options] [DIST_DIR]

Installs the swapdock binary and the host files it needs. The default
DIST_DIR is the invoking user's ~/swapdock-dist.

options:
  --dry-run       Print the plan, change nothing, do not require root
  --with-service  Also install install/systemd/swapdock.service (not started)
  --non-root      Run the API as the deploy user; install the privileged
                  apply broker and its hardening unit (implies --with-service)
  --with-backup-timer
                  Also install swapdock-backup.service/.timer (not enabled)
  --with-monitor-timer
                  Also install swapdock-monitor.service/.timer (not enabled)
  -h, --help      Show this help

This script never reloads nginx, starts the API, or restarts an app.
EOF
}

while (($#)); do
    case "$1" in
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --with-service)
            WITH_SERVICE=1
            shift
            ;;
        --with-backup-timer)
            WITH_BACKUP_TIMER=1
            shift
            ;;
        --with-monitor-timer)
            WITH_MONITOR_TIMER=1
            shift
            ;;
        --non-root)
            NON_ROOT=1
            WITH_SERVICE=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            printf 'unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
        *)
            [[ -z "$DIST" ]] || die "unexpected extra argument: $1"
            DIST="$1"
            shift
            ;;
    esac
done
if (($#)); then
    [[ -z "$DIST" ]] || die "unexpected extra argument: $1"
    DIST="$1"
fi

if [[ -z "$DIST" ]]; then
    if [[ -n "${SUDO_USER:-}" ]]; then
        DIST="/home/$SUDO_USER/swapdock-dist"
    else
        DIST="${HOME:-/root}/swapdock-dist"
    fi
fi

[[ -d "$DIST" ]] || die "dist dir missing: $DIST"
[[ -x "$DIST/swapdock" ]] || die "$DIST/swapdock missing or not executable"

shopt -s nullglob
app_templates=("$DIST"/apps/*.toml)

if ((DRY_RUN)); then
    say "dry run: no changes will be made"
    if "$DIST/swapdock" --version; then
        :
    else
        printf 'warning: %s --version failed; a real install would refuse\n' "$DIST/swapdock" >&2
    fi
    echo "dist:         $DIST"
    echo "install user: ${SUDO_USER:-$(id -un)} (a real run must be started with sudo)"
    echo "app templates: ${#app_templates[@]}"
    for cmd in nginx docker visudo; do
        if command -v "$cmd" >/dev/null 2>&1; then
            echo "found:        $cmd"
        else
            echo "warning:      $cmd not found; a real run would stop"
        fi
    done
    cat <<EOF

would:
  - back up /etc/nginx to /root/nginx-pre-swapdock-<timestamp>.tgz
  - install $DIST/swapdock -> /usr/local/bin/swapdock and the narrow root wrapper (.bak kept)
  - create /srv/swapdock/{apps,green,overrides,backups} and /var/log/swapdock
  - create the run log 0640, owned by the deploy user and group-readable
  - copy ${#app_templates[@]} app template(s) into /srv/swapdock/apps (never overwrite)
  - install nginx files: conf.d/00-swapdock-http.conf, conf.d/swapdock-ui.conf,
    snippets/proxy-common.conf, snippets/spa-fallback.conf (.bak kept)
  - install the built console from dist/ui to /srv/swapdock/ui
  - add the deploy user to adm (needed by verify; no-op if already a member)
  - write /etc/logrotate.d/swapdock (.bak kept); write the sudoers entry only in root mode
  - run nginx -t (test only; no reload)
EOF
    if ((WITH_SERVICE)); then
        if ((NON_ROOT)); then
            echo "  - add the deploy user to adm; Docker commands go through the broker"
            echo "  - make /srv/swapdock/green and /srv/swapdock/locks deploy-user-owned"
            echo "  - install swapdock-broker.service (root) and a non-root API unit"
            echo "  - create /etc/swapdock/api.env (0600) if missing; never rotate it"
            echo "    NOT enable or start either unit"
        else
            echo "  - install /etc/systemd/system/swapdock.service, then daemon-reload;"
            echo "    NOT enable and NOT start it (loopback only, no TLS yet)"
            echo "  - create /etc/swapdock/api.env (0600) with a random SWAPDOCK_API_TOKEN"
            echo "    if it is missing; never rotate an existing token"
        fi
    fi
    if ((WITH_BACKUP_TIMER)); then
        echo "  - install swapdock-backup.service/.timer, then daemon-reload;"
        echo "    NOT enable the timer (run a restore drill first)"
    fi
    if ((WITH_MONITOR_TIMER)); then
        echo "  - install swapdock-monitor.service/.timer, then daemon-reload;"
        echo "    NOT enable the timer"
    fi
    echo
    echo "dry run complete; nothing was written."
    exit 0
fi

[[ "$(id -u)" -eq 0 ]] || die "run as root: sudo bash $0 $DIST"
INSTALL_USER="${SUDO_USER:-}"
[[ -n "$INSTALL_USER" ]] || die "run via sudo, not as root directly"
id "$INSTALL_USER" >/dev/null 2>&1 || die "unknown user: $INSTALL_USER"
LOG_GROUP="$(id -gn "$INSTALL_USER")"

backup_once() {
    local path="$1"
    if [[ -e "$path" && ! -e "$path.bak" ]]; then
        cp -a -- "$path" "$path.bak"
        echo "backed up $path to $path.bak"
    fi
}

say "preconditions"
command -v nginx >/dev/null 2>&1 || die "nginx not installed"
command -v docker >/dev/null 2>&1 || die "docker not installed"
command -v visudo >/dev/null 2>&1 || die "visudo not installed"
[[ -f "$DIST/logrotate-swapdock" ]] || die "$DIST/logrotate-swapdock missing"
for name in 00-swapdock-http.conf proxy-common.conf spa-fallback.conf swapdock-ui.conf; do
    [[ -f "$DIST/nginx/$name" ]] || die "$DIST/nginx/$name missing"
done
if ((WITH_SERVICE)); then
    if ((NON_ROOT)); then
        [[ -f "$DIST/systemd/swapdock-nonroot.service" ]] || die "--non-root: $DIST/systemd/swapdock-nonroot.service missing"
        [[ -f "$DIST/systemd/swapdock-broker.service" ]] || die "--non-root: $DIST/systemd/swapdock-broker.service missing"
    else
        [[ -f "$DIST/systemd/swapdock.service" ]] || die "--with-service: $DIST/systemd/swapdock.service missing"
    fi
fi
if ((WITH_BACKUP_TIMER)); then
    [[ -f "$DIST/systemd/swapdock-backup.service" ]] || die "--with-backup-timer: $DIST/systemd/swapdock-backup.service missing"
    [[ -f "$DIST/systemd/swapdock-backup.timer" ]] || die "--with-backup-timer: $DIST/systemd/swapdock-backup.timer missing"
fi
if ((WITH_MONITOR_TIMER)); then
    [[ -f "$DIST/systemd/swapdock-monitor.service" ]] || die "--with-monitor-timer: $DIST/systemd/swapdock-monitor.service missing"
    [[ -f "$DIST/systemd/swapdock-monitor.timer" ]] || die "--with-monitor-timer: $DIST/systemd/swapdock-monitor.timer missing"
fi
"$DIST/swapdock" --version

say "back up /etc/nginx"
BACKUP="/root/nginx-pre-swapdock-$(date +%F-%H%M%S).tgz"
tar -czf "$BACKUP" -C / etc/nginx
echo "backup: $BACKUP"

say "install the binary"
backup_once /usr/local/bin/swapdock
install -m 0755 -- "$DIST/swapdock" /usr/local/bin/swapdock
install -d -m 0755 /usr/local/lib/swapdock
install -m 0755 -- "$DIST/swapdock-root" /usr/local/lib/swapdock/swapdock-root

say "create /srv/swapdock and /var/log/swapdock"
install -d -m 0755 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock
install -d -m 0755 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock/apps
install -d -m 0755 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock/overrides
install -d -m 0755 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock/backups
install -d -m 0755 -o root -g root /srv/swapdock/green
if ((${#app_templates[@]} == 0)); then
    echo "no apps/*.toml in the dist; registry left as is"
else
    # -n: never overwrite. Re-runs keep operator edits; new apps still land.
    cp -n -- "${app_templates[@]}" /srv/swapdock/apps/
fi
chown -R "$INSTALL_USER:$INSTALL_USER" /srv/swapdock/apps
# The run log is appended by unprivileged commands too, so the deploy user owns
# the file and its group can read it. touch, never truncate: this re-runs.
install -d -m 2770 -o root -g "$LOG_GROUP" /var/log/swapdock
touch /var/log/swapdock/swapdock.jsonl
# 0640, not 0664: the log stays operator data. The deploy user owns the file
# so unprivileged commands can still append; the group can read it for support.
chown "$INSTALL_USER:$LOG_GROUP" /var/log/swapdock/swapdock.jsonl
chmod 0640 /var/log/swapdock/swapdock.jsonl
mapfile -t registry_files < <(find /srv/swapdock/apps -maxdepth 1 -type f -name '*.toml' | sort)
echo "registry files: ${#registry_files[@]}"

say "install hand-written nginx files"
install -d -m 0755 /etc/nginx/snippets /etc/nginx/conf.d
install_nginx_file() {
    local name="$1"
    local dest="$2"
    backup_once "$dest"
    install -m 0644 -- "$DIST/nginx/$name" "$dest"
    echo "wrote $dest"
}
install_nginx_file 00-swapdock-http.conf /etc/nginx/conf.d/00-swapdock-http.conf
install_nginx_file swapdock-ui.conf /etc/nginx/conf.d/swapdock-ui.conf
install_nginx_file proxy-common.conf /etc/nginx/snippets/proxy-common.conf
install_nginx_file spa-fallback.conf /etc/nginx/snippets/spa-fallback.conf

if [[ -d "$DIST/ui" ]]; then
    say "install the console UI"
    install -d -m 0755 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock/ui
    cp -a "$DIST/ui/." /srv/swapdock/ui/
    chown -R "$INSTALL_USER:$INSTALL_USER" /srv/swapdock/ui
    echo "installed /srv/swapdock/ui"
fi

say "prove the config still passes"
nginx -t

if ((NON_ROOT)); then
    say "non-root mode: no passwordless sudo"
    if [[ -e /etc/sudoers.d/swapdock ]]; then
        backup_once /etc/sudoers.d/swapdock
        rm -f -- /etc/sudoers.d/swapdock
        echo "removed the old broad sudoers entry; use the broker / systemd units"
    fi
else
    say "passwordless sudo for the swapdock binary"
    backup_once /etc/sudoers.d/swapdock
    sudoers_tmp="$(mktemp /etc/sudoers.d/.swapdock.XXXXXX)"
    printf '%s\n' "$INSTALL_USER ALL=(root) NOPASSWD: /usr/local/lib/swapdock/swapdock-root" >"$sudoers_tmp"
    chmod 0440 "$sudoers_tmp"
    if ! visudo -c -f "$sudoers_tmp" >/dev/null 2>&1; then
        rm -f -- "$sudoers_tmp"
        die "generated sudoers entry is invalid; left the old file untouched"
    fi
    mv -f -- "$sudoers_tmp" /etc/sudoers.d/swapdock
    chmod 0440 /etc/sudoers.d/swapdock
    visudo -c -f /etc/sudoers.d/swapdock >/dev/null
fi

say "log rotation for the run log"
backup_once /etc/logrotate.d/swapdock
logrotate_tmp="$(mktemp /etc/logrotate.d/.swapdock.XXXXXX)"
# Substituted, not copied: the recreated log must keep the deploy user as owner
# (so unprivileged appends still work) and the deploy group as group.
sed -e "s/@LOG_USER@/$INSTALL_USER/" -e "s/@LOG_GROUP@/$LOG_GROUP/" \
    "$DIST/logrotate-swapdock" >"$logrotate_tmp"
chmod 0644 "$logrotate_tmp"
mv -f -- "$logrotate_tmp" /etc/logrotate.d/swapdock

say "let $INSTALL_USER read the nginx access log"
# verify reads the front-door access log, which is www-data:adm 0640.
if id -nG "$INSTALL_USER" | grep -qw adm; then
    echo "$INSTALL_USER is already in adm"
else
    usermod -aG adm "$INSTALL_USER"
    echo "added $INSTALL_USER to adm; log out and back in for verify to use it"
fi

if ((NON_ROOT)); then
    say "prepare the non-root runtime"
    if id -nG "$INSTALL_USER" | grep -qw docker; then
        die "non-root mode refuses a deploy user in the docker group; remove it from docker first (it would defeat the broker)"
    fi
    install -d -m 2770 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock/green
    install -d -m 2770 -o "$INSTALL_USER" -g "$INSTALL_USER" /srv/swapdock/locks
    install -d -m 0700 -o root -g root /var/lib/swapdock/docker
    install -d -m 0755 -o root -g root /run/swapdock
fi

if ((WITH_SERVICE)); then
    say "install the systemd unit (not started)"
    # The token file is shared by the API and the broker. Generate it once.
    install -d -m 0755 -o root -g root /etc/swapdock
    if [[ ! -e /etc/swapdock/api.env ]]; then
        token="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
        broker_token="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
        umask 077
        printf 'SWAPDOCK_API_TOKEN=%s\nSWAPDOCK_BROKER_TOKEN=%s\n' \
            "$token" "$broker_token" >/etc/swapdock/api.env
        unset token broker_token
        chown root:root /etc/swapdock/api.env
        chmod 0600 /etc/swapdock/api.env
        echo "generated /etc/swapdock/api.env with a random API token (0600)"
    else
        echo "kept existing /etc/swapdock/api.env (token not rotated)"
    fi
    if ((NON_ROOT)); then
        backup_once /etc/systemd/system/swapdock-broker.service
        backup_once /etc/systemd/system/swapdock-nonroot.service
        install -m 0644 -- "$DIST/systemd/swapdock-broker.service" /etc/systemd/system/swapdock-broker.service
        sed "s/@INSTALL_USER@/$INSTALL_USER/g" "$DIST/systemd/swapdock-nonroot.service"             > /etc/systemd/system/swapdock.service
        chmod 0644 /etc/systemd/system/swapdock.service
        echo "installed swapdock-broker.service and non-root swapdock.service"
    else
        backup_once /etc/systemd/system/swapdock.service
        install -m 0644 -- "$DIST/systemd/swapdock.service" /etc/systemd/system/swapdock.service
        echo "installed /etc/systemd/system/swapdock.service (root)"
    fi
    if command -v systemctl >/dev/null 2>&1; then
        if ! systemctl daemon-reload; then
            echo "warning: systemctl daemon-reload failed; run it manually" >&2
        fi
    fi
    echo "NOT enabled and NOT started on purpose."
    echo "The API has token auth plus a single-admin HttpOnly session + CSRF and"
    echo "is bound to 127.0.0.1. It has no TLS; put an authenticated TLS proxy or"
    echo "Cloudflare Tunnel in front. Read SECURITY.md and docs/tls.md first."
    echo "Read the token with: sudo cat /etc/swapdock/api.env"
    if ((NON_ROOT)); then
        echo "Start the broker before the API:"
        echo "  sudo systemctl enable --now swapdock-broker.service"
        echo "  sudo systemctl enable --now swapdock.service"
        echo "The API runs as $INSTALL_USER; the broker (root) writes nginx and"
        echo "executes allowlisted Docker commands. The API needs no docker group,"
        echo "and non-root mode installs no passwordless sudo entry."
    else
        echo "Start the API (root): sudo systemctl enable --now swapdock.service"
    fi
fi

if ((WITH_BACKUP_TIMER)); then
    say "install the daily backup timer (not enabled)"
    backup_once /etc/systemd/system/swapdock-backup.service
    backup_once /etc/systemd/system/swapdock-backup.timer
    install -m 0644 -- "$DIST/systemd/swapdock-backup.service" /etc/systemd/system/swapdock-backup.service
    install -m 0644 -- "$DIST/systemd/swapdock-backup.timer" /etc/systemd/system/swapdock-backup.timer
    if command -v systemctl >/dev/null 2>&1; then
        if ! systemctl daemon-reload; then
            echo "warning: systemctl daemon-reload failed; run it manually" >&2
        fi
    fi
    echo "installed swapdock-backup.service and swapdock-backup.timer"
    echo "NOT enabled on purpose. Enable after a tested restore:"
    echo "  sudo systemctl enable --now swapdock-backup.timer"
fi

if ((WITH_MONITOR_TIMER)); then
    say "install the five-minute monitor timer (not enabled)"
    backup_once /etc/systemd/system/swapdock-monitor.service
    backup_once /etc/systemd/system/swapdock-monitor.timer
    install -m 0644 -- "$DIST/systemd/swapdock-monitor.service" /etc/systemd/system/swapdock-monitor.service
    install -m 0644 -- "$DIST/systemd/swapdock-monitor.timer" /etc/systemd/system/swapdock-monitor.timer
    if command -v systemctl >/dev/null 2>&1; then
        if ! systemctl daemon-reload; then
            echo "warning: systemctl daemon-reload failed; run it manually" >&2
        fi
    fi
    echo "installed swapdock-monitor.service and swapdock-monitor.timer"
    echo "NOT enabled on purpose. Enable after setting SWAPDOCK_TLS_CERTS and"
    echo "SWAPDOCK_WEBHOOK_URL in /etc/swapdock/api.env:"
    echo "  sudo systemctl enable --now swapdock-monitor.timer"
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
