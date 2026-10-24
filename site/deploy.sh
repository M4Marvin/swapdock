#!/usr/bin/env bash
# deploy.sh — build the site and deploy it as a swapdock static app.
#
#   ./deploy.sh [--dry-run] [--skip-dns] [--release <hex>]
#
# swapdock's static strategy owns the cutover:
#
#   1. build the site, publish a release under /srv/www/swapdock-site/releases
#   2. add the hostname to the cloudflared ingress and validate it
#   3. install the registry entry and validate the estate against the tunnel
#   4. swapdock up swapdock-site --release <release>
#      verify the release -> flip the `current` symlink -> render + apply nginx
#      -> probe the front door -> commit the registry
#   5. restart cloudflared, create the DNS CNAME, verify local + public
#
# Overrides:
#   SITE_HOSTNAME=swapdock.m4marvin.com
#   SITE_FRONT_PORT=8016
#   SITE_SLOT=10
#   SWAPDOCK_REGISTRY=/srv/swapdock/apps
#   SITE_ROOT=/srv/www/swapdock-site
#   CLOUDFLARED_CONFIG=$HOME/.cloudflared/swapdock-site.yml
#   TUNNEL_SERVICE=swapdock-site-tunnel.service
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"

SITE_APP="swapdock-site"
SITE_HOSTNAME="${SITE_HOSTNAME:-swapdock.m4marvin.com}"
SITE_FRONT_PORT="${SITE_FRONT_PORT:-8016}"
SITE_SLOT="${SITE_SLOT:-10}"
SWAPDOCK_REGISTRY="${SWAPDOCK_REGISTRY:-/srv/swapdock/apps}"
SITE_ROOT="${SITE_ROOT:-/srv/www/swapdock-site}"
CLOUDFLARED_CONFIG="${CLOUDFLARED_CONFIG:-$HOME/.cloudflared/swapdock-site.yml}"
TUNNEL_SERVICE="${TUNNEL_SERVICE:-swapdock-site-tunnel.service}"
REGISTRY_FILE="$SWAPDOCK_REGISTRY/$SITE_APP.toml"

dry=0
skip_dns=0
release=""

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) dry=1 ;;
        --skip-dns) skip_dns=1 ;;
        --release)
            release="${2:?--release needs a value}"
            shift
            ;;
        -h|--help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

step() { printf '\n== %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing command: $1"; }

for c in pnpm swapdock cloudflared sudo rsync curl git python3 ss; do need "$c"; done
[ "$(id -u)" -ne 0 ] || die "run this as your normal user; it uses sudo where needed"
[ -f "$CLOUDFLARED_CONFIG" ] || die "cloudflared config not found: $CLOUDFLARED_CONFIG"
[ -d "$SWAPDOCK_REGISTRY" ] || die "registry not found: $SWAPDOCK_REGISTRY"

TUNNEL_ID="$(awk '/^tunnel:/{print $2; exit}' "$CLOUDFLARED_CONFIG")"
[ -n "$TUNNEL_ID" ] || die "no tunnel id in $CLOUDFLARED_CONFIG"

[[ "$SITE_HOSTNAME" =~ ^[a-z0-9][a-z0-9.-]*[a-z0-9]$ ]] || die "invalid hostname: $SITE_HOSTNAME"
[ "$SITE_FRONT_PORT" -ge 1024 ] && [ "$SITE_FRONT_PORT" -le 65535 ] || die "invalid front port"
{ [ "$SITE_FRONT_PORT" -lt 9000 ] || [ "$SITE_FRONT_PORT" -gt 9255 ]; } || die "front port is in the reserved back range"

if [ -z "$release" ]; then
    if git rev-parse --git-dir >/dev/null 2>&1; then
        release="$(git rev-parse --short=12 HEAD)"
        if [ -n "$(git status --porcelain -- . 2>/dev/null)" ]; then
            # A dirty tree still needs a hex release, so append a hex timestamp.
            release="${release}$(printf '%x' "$(date +%s)")"
        fi
    else
        release="$(printf '%x' "$(date +%s)")"
    fi
fi
[[ "$release" =~ ^[0-9a-f]{7,64}$ ]] || die "release must be hex (7-64 chars), got: $release"

if [ "$dry" -eq 1 ]; then
    cat <<PLAN
== plan (no changes)

  host      $SITE_HOSTNAME
  front     :$SITE_FRONT_PORT  (host nginx, loopback)
  slot      $SITE_SLOT
  release   $release
  root      $SITE_ROOT/current -> releases/$release
  registry  $REGISTRY_FILE
  tunnel    $TUNNEL_ID  ($CLOUDFLARED_CONFIG, user service $TUNNEL_SERVICE)

  1. VITE_SITE_URL=https://$SITE_HOSTNAME pnpm build && pnpm check
  2. publish dist/client/ -> $SITE_ROOT/releases/$release/   (sudo only if the root needs creating)
  3. add "$SITE_HOSTNAME -> http://localhost:$SITE_FRONT_PORT" to the tunnel ingress
  4. install the registry entry; validate the estate + tunnel
  5. sudo swapdock --registry $SWAPDOCK_REGISTRY up $SITE_APP --release $release --force
     (verify release, flip current, render + apply nginx, probe front, commit)
  6. systemctl --user restart $TUNNEL_SERVICE
     cloudflared tunnel route dns $TUNNEL_ID $SITE_HOSTNAME
  7. curl http://127.0.0.1:$SITE_FRONT_PORT/ and /path/ (Host), then https://$SITE_HOSTNAME/
PLAN
    exit 0
fi

# During a redeploy nginx already listens on the front port. Anything else must not.
if ss -ltn "sport = :$SITE_FRONT_PORT" | tail -n +2 | grep -q .; then
    if [ -f "$REGISTRY_FILE" ] && grep -q "^front_port = $SITE_FRONT_PORT$" "$REGISTRY_FILE"; then
        :
    else
        die "port $SITE_FRONT_PORT is already in use by something else"
    fi
fi

step "build $release"
VITE_SITE_URL="https://$SITE_HOSTNAME" pnpm build
pnpm check

step "publish release"
if mkdir -p "$SITE_ROOT/releases" 2>/dev/null; then
    rsync -a --delete "dist/client/" "$SITE_ROOT/releases/$release/"
    chmod -R a+rX "$SITE_ROOT/releases/$release"
else
    sudo install -d -m 0755 -o "$(id -un)" -g "$(id -gn)" "$SITE_ROOT" "$SITE_ROOT/releases"
    sudo rsync -a --delete "dist/client/" "$SITE_ROOT/releases/$release/"
    sudo chown -R "$(id -un):$(id -gn)" "$SITE_ROOT/releases/$release"
    sudo chmod -R a+rX "$SITE_ROOT/releases/$release"
fi
if [ -e "$SITE_ROOT/current" ] && [ ! -L "$SITE_ROOT/current" ]; then
    die "$SITE_ROOT/current exists and is not a symlink"
fi

step "tunnel ingress"
backup="$CLOUDFLARED_CONFIG.bak-swapdock-site-$(date -u +%Y%m%dT%H%M%SZ)"
cp -a "$CLOUDFLARED_CONFIG" "$backup"
python3 - "$CLOUDFLARED_CONFIG" "$SITE_HOSTNAME" "$SITE_FRONT_PORT" <<'PY'
import re
import sys

path, host, port = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(path).read()
match = re.search(rf"^  - hostname: {re.escape(host)}\n    service: (\S+)$", text, re.M)
if match:
    expected = f"http://localhost:{port}"
    if match.group(1) != expected:
        sys.exit(f"{host} is already routed to {match.group(1)}, not {expected}")
    print(f"{host} is already routed to {expected}")
else:
    marker = "  - service: http_status:404"
    if marker not in text:
        sys.exit("cloudflared config has no catch-all (http_status:404)")
    block = f"  - hostname: {host}\n    service: http://localhost:{port}\n"
    text = text.replace(marker, block + marker, 1)
    open(path, "w").write(text)
    print(f"added {host} -> http://localhost:{port}")
PY
cloudflared --config "$CLOUDFLARED_CONFIG" tunnel ingress validate

step "registry entry"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
sed -e "s|^hostnames = .*|hostnames = [\"$SITE_HOSTNAME\"]|" \
    -e "s|^front_port = .*|front_port = $SITE_FRONT_PORT|" \
    -e "s|^slot = .*|slot = $SITE_SLOT|" \
    -e "s|^root = .*|root = \"$SITE_ROOT/current\"|" \
    deploy/swapdock-site.toml > "$tmp"
if [ -f "$REGISTRY_FILE" ] && ! cmp -s "$tmp" "$REGISTRY_FILE"; then
    cp -a "$REGISTRY_FILE" "$REGISTRY_FILE.bak-$(date -u +%Y%m%dT%H%M%SZ)"
fi
if [ -w "$SWAPDOCK_REGISTRY" ]; then
    install -m 0640 "$tmp" "$REGISTRY_FILE"
else
    sudo install -m 0640 -o "$(id -un)" -g "$(id -gn)" "$tmp" "$REGISTRY_FILE"
fi

step "validate the estate"
out="$(swapdock --registry "$SWAPDOCK_REGISTRY" validate 2>&1)" || {
    printf '%s\n' "$out"
    die "registry validation failed"
}
printf '%s\n' "$out"
printf '%s\n' "$out" | grep -q "0 error(s)" || die "registry validation reported errors"

step "deploy with swapdock"
sudo swapdock --registry "$SWAPDOCK_REGISTRY" up "$SITE_APP" --release "$release" --force

step "restart tunnel connector"
systemctl --user restart "$TUNNEL_SERVICE"
for _ in 1 2 3 4 5 6 7 8 9 10; do
    systemctl --user is-active --quiet "$TUNNEL_SERVICE" && break
    sleep 1
done
systemctl --user is-active --quiet "$TUNNEL_SERVICE" || die "$TUNNEL_SERVICE did not come back up"

if [ "$skip_dns" -eq 0 ]; then
    step "dns"
    if ! cloudflared tunnel route dns "$TUNNEL_ID" "$SITE_HOSTNAME"; then
        printf 'warning: DNS was not created automatically. Add a proxied CNAME:\n' >&2
        printf '  %s -> %s.cfargotunnel.com\n' "$SITE_HOSTNAME" "$TUNNEL_ID" >&2
    fi
else
    step "dns skipped"
fi

step "verify"
local_code="$(curl -sS -o /dev/null -w '%{http_code}' -H "Host: $SITE_HOSTNAME" "http://127.0.0.1:$SITE_FRONT_PORT/" || true)"
deep_code="$(curl -sS -o /dev/null -w '%{http_code}' -H "Host: $SITE_HOSTNAME" "http://127.0.0.1:$SITE_FRONT_PORT/path/" || true)"
[ "$local_code" = "200" ] && [ "$deep_code" = "200" ] || die "local front door returned /=$local_code /path/=$deep_code"
echo "local  200  http://127.0.0.1:$SITE_FRONT_PORT/ and /path/ (Host: $SITE_HOSTNAME)"

public_code=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
    public_code="$(curl -sSL --max-time 15 -o /dev/null -w '%{http_code}' "https://$SITE_HOSTNAME/" || true)"
    [ "$public_code" = "200" ] && break
    sleep 3
done
if [ "$public_code" = "200" ]; then
    echo "public 200  https://$SITE_HOSTNAME/"
else
    echo "public $public_code  https://$SITE_HOSTNAME/" >&2
    echo "The deploy succeeded locally. DNS may still be propagating, or the CNAME is missing." >&2
fi

step "deployed"
cat <<SUMMARY
  site      https://$SITE_HOSTNAME/
  release   $release
  rollback  sudo swapdock --registry $SWAPDOCK_REGISTRY rollback $SITE_APP
  status    swapdock --registry $SWAPDOCK_REGISTRY status $SITE_APP
SUMMARY
