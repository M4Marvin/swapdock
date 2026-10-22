# TLS and the console proxy

**Status: required for a public deployment; certificates are not automated yet.**
The API can terminate TLS itself when given a certificate/key; the console is
still served by nginx, so the same-origin production shape is a TLS reverse
proxy in front of the loopback API with the SPA served from that origin. No
ACME client is built in.

```text
internet -> TLS proxy (nginx/Caddy/Cloudflare) -> /api -> 127.0.0.1:8088
                                             -> /    -> SPA files
```

## Built-in API TLS

If you terminate TLS on the API listener instead of a proxy, supply both files:

```bash
swapdock serve \
  --bind 127.0.0.1:8088 \
  --tls-cert /etc/letsencrypt/live/api.example.com/fullchain.pem \
  --tls-key  /etc/letsencrypt/live/api.example.com/privkey.pem
```

`--tls-cert` and `--tls-key` must be supplied together; missing files fail
before the listener binds. The bind must be an IP:port in TLS mode. This does
not obtain or renew the certificate and does not make the console same-origin;
use the reverse-proxy shape below for the console, or put the API behind a
proxy as well. The HTTP listener remains available when both flags are absent.

## Cloudflare Tunnel (no ACME)

For a one-user tool, Cloudflare Tunnel is the supported TLS path without
exposing a public port or running an ACME client. It was exercised against
`console.m4marvin.com` with a same-origin `/api` proxy:

```bash
cloudflared tunnel create swapdock
cloudflared tunnel route dns swapdock console.example.com
```

`~/.cloudflared/<tunnel-id>.yml`:

```yaml
tunnel: <tunnel-id>
credentials-file: /home/<user>/.cloudflared/<tunnel-id>.json
ingress:
  - hostname: console.example.com
    service: http://127.0.0.1:8080
  - service: http_status:404
```

Run it as the operator (or install the standard `cloudflared` service). The
installed nginx serves `/srv/swapdock/ui` on `127.0.0.1:8080` and proxies
`/api` to the loopback API; Cloudflare
terminates TLS, redirects HTTP to HTTPS, and the same-origin cookie session
works. Add HSTS locally so it is present regardless of zone settings:

```nginx
add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
```

The tunnel is public. Keep the token gate/session and do not put a weak token on
a public hostname.

## Why same-origin

The console fetches `/api` and opens `EventSource` streams under `/api/events`.
Serving both from one HTTPS origin:

- avoids mixed content and CORS;
- lets the SSE query token remain on the same origin;
- means one certificate and one auth boundary.

Do **not** put `100.x:8088` (or any API address) in `web/public/config.json`.
Leave it at the default `/api`.

## nginx example

Use [install/nginx/swapdock-ui-tls.conf.example](../install/nginx/swapdock-ui-tls.conf.example).
It:

- redirects HTTP to HTTPS (keeping the ACME path);
- terminates TLS;
- proxies `/api/` to `127.0.0.1:8088` without buffering SSE;
- serves the console from `/srv/www/swapdock-ui/current`.

Replace the hostname and certificate paths, then:

```bash
sudo nginx -t
sudo systemctl reload nginx
```

Get a certificate with your normal tool, for example:

```bash
sudo certbot --nginx -d console.example.com
```

## API side

```bash
# /etc/swapdock/api.env, mode 0600
SWAPDOCK_API_TOKEN=<64 hex characters>
```

The API must bind loopback (`--bind 127.0.0.1:8088`, the default). The proxy is
the only public listener. The token gate in the console asks the operator for
this value; it is stored in `sessionStorage`, not `localStorage`.

## Still open

- automatic ACME/certificate renewal and a `swapdock setup --tls` path;
- a first-class user login/session (the current credential is one shared token);
- an audit actor and token rotation;
- an independent TLS/security review.

Until those land, this is a documented deployment shape, not a one-command
production setup. See [LAUNCH.md](../LAUNCH.md) Gate 0.4/G0.5.
