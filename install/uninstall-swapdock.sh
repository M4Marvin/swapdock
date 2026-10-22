#!/usr/bin/env bash
# uninstall-swapdock.sh — remove what install-swapdock.sh added.
#
#   sudo bash uninstall-swapdock.sh [--dry-run] [--purge]
#
# Removes:
#   - /usr/local/bin/swapdock
#   - /usr/local/lib/swapdock/swapdock-root
#   - /etc/sudoers.d/swapdock
#   - /etc/logrotate.d/swapdock
#   - /etc/nginx/conf.d/00-swapdock-http.conf
#     /etc/nginx/conf.d/swapdock-ui.conf
#     /etc/nginx/snippets/proxy-common.conf
#     /etc/nginx/snippets/spa-fallback.conf
#   - /srv/swapdock/ui (the built console)
#     (each is restored from <file>.bak when a backup exists)
#   - /etc/systemd/system/swapdock.service, if present (stopped and disabled)
#   - /etc/systemd/system/swapdock-backup.service and .timer, if present
#   - /etc/systemd/system/swapdock-monitor.service and .timer, if present
#   - /etc/systemd/system/swapdock-broker.service, if present
#
# Keeps unless --purge:
#   - /srv/swapdock (the registry, overrides and backups)
#   - /var/log/swapdock (the run log)
#
# Never stops or removes app containers, and never removes the generated
# /etc/nginx/conf.d/front-door.conf. That file is written by `apply`, not by
# the installer, and removing it would break live app routing on the next
# nginx reload. The run ends with a warning when removing the snippets leaves
# that generated file referencing files that no longer exist.
#
# Idempotent: missing files are reported and skipped. --dry-run prints the
# plan, changes nothing, and does not need root.
set -euo pipefail

DRY_RUN=0
PURGE=0

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

say() {
    printf '== %s ==\n' "$*"
}

usage() {
    cat <<'EOF'
usage: sudo bash uninstall-swapdock.sh [options]

Removes the swapdock binary and the host files install-swapdock.sh added.
Keeps /srv/swapdock and /var/log/swapdock unless --purge.
Never stops app containers.

options:
  --dry-run  Print the plan, change nothing, do not require root
  --purge    Also remove /srv/swapdock and /var/log/swapdock
  -h, --help Show this help
EOF
}

while (($#)); do
    case "$1" in
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --purge)
            PURGE=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

if ((DRY_RUN)); then
    say "dry run: no changes will be made"
else
    [[ "$(id -u)" -eq 0 ]] || die "run as root: sudo bash $0"
fi

remove_file() {
    local path="$1"
    if [[ ! -e "$path" && ! -L "$path" ]]; then
        echo "absent:  $path"
        return
    fi
    if ((DRY_RUN)); then
        echo "would remove: $path"
    else
        rm -f -- "$path"
        echo "removed: $path"
    fi
}

restore_or_remove() {
    local path="$1"
    if [[ -e "$path.bak" ]]; then
        if ((DRY_RUN)); then
            echo "would restore: $path (from $path.bak)"
        else
            mv -f -- "$path.bak" "$path"
            echo "restored: $path (from $path.bak)"
        fi
        return
    fi
    remove_file "$path"
}

remove_tree() {
    local path="$1"
    if [[ ! -d "$path" ]]; then
        echo "absent:  $path"
        return
    fi
    if ((DRY_RUN)); then
        echo "would remove tree: $path"
    else
        rm -rf -- "$path"
        echo "removed tree: $path"
    fi
}

say "stop the optional API unit"
UNIT="/etc/systemd/system/swapdock.service"
if [[ -e "$UNIT" ]]; then
    if command -v systemctl >/dev/null 2>&1; then
        if ((DRY_RUN)); then
            echo "would stop and disable: swapdock.service"
        else
            systemctl disable --now swapdock.service >/dev/null 2>&1 || true
            echo "stopped and disabled: swapdock.service"
        fi
    fi
    remove_file "$UNIT"
    if ((DRY_RUN)); then
        echo "would run: systemctl daemon-reload"
    elif command -v systemctl >/dev/null 2>&1; then
        if ! systemctl daemon-reload; then
            echo "warning: systemctl daemon-reload failed; run it manually" >&2
        fi
    fi
else
    echo "absent:  $UNIT"
fi
# The token file was created by --with-service. Remove it with the unit: a
# stale credential for a service that no longer exists is not useful.
remove_file /etc/swapdock/api.env

say "stop the optional backup timer"
BACKUP_TIMER="/etc/systemd/system/swapdock-backup.timer"
BACKUP_SERVICE="/etc/systemd/system/swapdock-backup.service"
if [[ -e "$BACKUP_TIMER" || -e "$BACKUP_SERVICE" ]]; then
    if ((DRY_RUN)); then
        echo "would disable: swapdock-backup.timer"
    elif command -v systemctl >/dev/null 2>&1; then
        systemctl disable --now swapdock-backup.timer >/dev/null 2>&1 || true
        echo "disabled: swapdock-backup.timer"
    fi
    remove_file "$BACKUP_TIMER"
    remove_file "$BACKUP_SERVICE"
    if ((DRY_RUN)); then
        echo "would run: systemctl daemon-reload"
    elif command -v systemctl >/dev/null 2>&1; then
        systemctl daemon-reload >/dev/null 2>&1 || true
    fi
else
    echo "absent:  $BACKUP_TIMER"
fi

say "stop the optional privileged broker"
BROKER_UNIT="/etc/systemd/system/swapdock-broker.service"
if [[ -e "$BROKER_UNIT" ]]; then
    if ((DRY_RUN)); then
        echo "would stop and disable: swapdock-broker.service"
    elif command -v systemctl >/dev/null 2>&1; then
        systemctl disable --now swapdock-broker.service >/dev/null 2>&1 || true
        echo "stopped and disabled: swapdock-broker.service"
    fi
    remove_file "$BROKER_UNIT"
else
    echo "absent:  $BROKER_UNIT"
fi

say "stop the optional monitor timer"
MONITOR_TIMER="/etc/systemd/system/swapdock-monitor.timer"
MONITOR_SERVICE="/etc/systemd/system/swapdock-monitor.service"
if [[ -e "$MONITOR_TIMER" || -e "$MONITOR_SERVICE" ]]; then
    if ((DRY_RUN)); then
        echo "would disable: swapdock-monitor.timer"
    elif command -v systemctl >/dev/null 2>&1; then
        systemctl disable --now swapdock-monitor.timer >/dev/null 2>&1 || true
        echo "disabled: swapdock-monitor.timer"
    fi
    remove_file "$MONITOR_TIMER"
    remove_file "$MONITOR_SERVICE"
    if ((DRY_RUN)); then
        echo "would run: systemctl daemon-reload"
    elif command -v systemctl >/dev/null 2>&1; then
        systemctl daemon-reload >/dev/null 2>&1 || true
    fi
else
    echo "absent:  $MONITOR_TIMER"
fi

say "remove the binary and sudo entry"
remove_file /usr/local/bin/swapdock
remove_file /usr/local/lib/swapdock/swapdock-root
remove_file /etc/sudoers.d/swapdock

say "remove log rotation"
remove_file /etc/logrotate.d/swapdock

say "remove the hand-written nginx files (restore .bak when present)"
restore_or_remove /etc/nginx/conf.d/00-swapdock-http.conf
restore_or_remove /etc/nginx/conf.d/swapdock-ui.conf
restore_or_remove /etc/nginx/snippets/proxy-common.conf
restore_or_remove /etc/nginx/snippets/spa-fallback.conf

say "remove the packaged console"
if [[ -d /srv/swapdock/ui ]]; then
    if ((DRY_RUN)); then
        echo "would remove: /srv/swapdock/ui"
    else
        rm -rf -- /srv/swapdock/ui
        echo "removed: /srv/swapdock/ui"
    fi
else
    echo "absent:  /srv/swapdock/ui"
fi

say "keep or purge state"
if ((PURGE)); then
    remove_tree /srv/swapdock
    remove_tree /var/log/swapdock
else
    echo "kept:    /srv/swapdock (registry, overrides, backups)"
    echo "kept:    /var/log/swapdock (run log)"
    echo "use --purge to remove both"
fi

say "check what remains"
if [[ -e /etc/nginx/conf.d/front-door.conf ]]; then
    echo "warning: kept /etc/nginx/conf.d/front-door.conf"
    echo "         it is generated by 'swapdock apply', not by the installer."
    echo "         If it includes snippet files that were just removed, a later"
    echo "         nginx reload will fail; running nginx keeps its loaded config."
    echo "         Back it up and remove or replace it before reloading nginx."
fi
if command -v nginx >/dev/null 2>&1; then
    if ((DRY_RUN)); then
        echo "would run: nginx -t (test only; no reload)"
    elif nginx -t >/dev/null 2>&1; then
        echo "nginx -t: still passes"
    else
        echo "nginx -t: fails now; inspect the error before any reload."
        echo "          A generated front-door.conf that references removed"
        echo "          snippets is a common cause. Running nginx keeps its"
        echo "          loaded config; no reload happened."
    fi
fi
if [[ -e /usr/local/bin/swapdock.bak ]]; then
    echo "kept:    /usr/local/bin/swapdock.bak (previous binary; not restored)"
fi
if [[ -e /etc/sudoers.d/swapdock.bak ]]; then
    echo "kept:    /etc/sudoers.d/swapdock.bak"
fi
if [[ -e /etc/logrotate.d/swapdock.bak ]]; then
    echo "kept:    /etc/logrotate.d/swapdock.bak"
fi

say "done"
echo "App containers were not stopped and no nginx reload was performed."
